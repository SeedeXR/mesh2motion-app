/**
 * The Three.js viewport.
 *
 * Everything decidable without a GPU lives in `model.ts`; this is the part that
 * needs a screen, and it is kept thin on purpose because it is the part no test
 * can look at.
 *
 * # Rendering is event-driven
 *
 * There is no permanent `requestAnimationFrame` loop. A frame is drawn when
 * something changes — the camera moved, the model changed, the canvas resized —
 * so an idle viewport costs no CPU and no GPU (todo P4-5). This is far easier
 * to build in than to retrofit, because a always-on loop lets state changes go
 * unannounced and everything quietly depends on that.
 *
 * Orbit damping is off for the same reason: damping needs a frame every tick
 * after the input stops, which is exactly the idle cost being avoided.
 */

import {
  AmbientLight,
  AnimationMixer,
  Box3,
  BufferAttribute,
  BufferGeometry,
  Color,
  DirectionalLight,
  GridHelper,
  LineBasicMaterial,
  LineSegments,
  type Material,
  Mesh,
  MeshBasicMaterial,
  type Object3D,
  PerspectiveCamera,
  Raycaster,
  Scene,
  SkeletonHelper,
  SphereGeometry,
  Vector2,
  Vector3,
  WebGLRenderer
} from 'three'
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js'
import { TransformControls } from 'three/examples/jsm/controls/TransformControls.js'
import {
  applyFraming,
  findClip,
  frameBounds,
  hasVertexColors,
  parseAnimated,
  parseModel,
  skeletonSegments,
  withJointMoved,
  type ModelContents
} from './model'

const FOV_DEGREES = 45

/** A mounted viewport. */
export interface Viewport {
  /** The canvas to place in the DOM. */
  readonly canvas: HTMLCanvasElement
  /** Draws a `.glb` from the bulk channel, replacing whatever was shown. */
  show(data: ArrayBuffer): Promise<ModelContents>
  /**
   * Draws a fitted template skeleton over the model, replacing any previous one.
   *
   * Separate from the glTF skeleton `show` draws: this one arrives as bare
   * positions and parent indices, with no scene graph for `SkeletonHelper` to
   * attach to.
   */
  showFittedSkeleton(
    positions: readonly (readonly [number, number, number])[],
    parents: readonly (number | null)[],
    onEdit?: (positions: ReadonlyArray<readonly [number, number, number]>) => void
  ): void
  /**
   * Plays a clip from an animated `.glb`, replacing whatever was shown.
   *
   * Returns the clip's duration in seconds, or `null` when the file carries no
   * clip by that name. Rendering runs a frame loop only while a clip is
   * playing (see `dispose`/`stop`), so an idle viewport still costs nothing.
   */
  playAnimated(data: ArrayBuffer, clipName: string): Promise<number | null>
  /** Stops playback and returns to the event-driven, zero-idle-cost renderer. */
  stop(): void
  /**
   * Draws a weight-paint overlay: a colour-baked model shown with its vertex
   * colours, replacing whatever was on screen. Returns to `showFittedSkeleton`
   * territory when the user leaves; call `show` again to restore the plain mesh.
   */
  showOverlay(data: ArrayBuffer): Promise<void>
  /** Frames the current model again, e.g. after the panel layout changed. */
  reframe(): void
  /** Releases the GPU resources this viewport holds. */
  dispose(): void
}

/** Creates a viewport. The canvas is returned unmounted. */
export function createViewport(): Viewport {
  const renderer = new WebGLRenderer({ antialias: true })
  // Above 2x the cost grows faster than anyone can see the difference, and a
  // 3x display would otherwise render nine times the pixels.
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2))

  const scene = new Scene()
  scene.background = new Color(0x14161a)

  const camera = new PerspectiveCamera(FOV_DEGREES, 1, 0.01, 100)
  const controls = new OrbitControls(camera, renderer.domElement)
  controls.enableDamping = false

  scene.add(new AmbientLight(0xffffff, 1.2))
  const key = new DirectionalLight(0xffffff, 1.8)
  key.position.set(2, 4, 3)
  scene.add(key)

  const grid = new GridHelper(4, 16, 0x2a2f37, 0x1e2228)
  scene.add(grid)

  let model: Object3D | null = null
  let skeleton: SkeletonHelper | null = null
  let fittedSkeleton: LineSegments | null = null

  // Bone-placement editing (Fit step). When `showFittedSkeleton` is given an
  // edit callback, a draggable handle sits on each joint and a translate gizmo
  // attaches to whichever the user clicks. The edited positions feed straight
  // back to binding — the callback is how they get there.
  let jointHandles: Mesh[] = []
  let handleGeometry: SphereGeometry | null = null
  let handleMaterial: MeshBasicMaterial | null = null
  let gizmo: TransformControls | null = null
  let editPositions: Array<[number, number, number]> = []
  let editParents: readonly (number | null)[] = []
  let onJointEdit: ((positions: ReadonlyArray<readonly [number, number, number]>) => void) | null =
    null
  const raycaster = new Raycaster()
  const pointer = new Vector2()

  // Playback state. The animated model is a separate object from `model` — the
  // imported mesh — so playing a clip does not disturb what the earlier steps
  // put on screen; it is removed again on `stop`.
  let animated: Object3D | null = null
  let overlay: Object3D | null = null
  let mixer: AnimationMixer | null = null
  // Elapsed-time tracked with performance.now() rather than three's Clock,
  // which is deprecated. `lastFrame` is the timestamp the mixer last advanced
  // from; the delta between frames is what it needs.
  let lastFrame = 0
  let playing = false
  let bounds = new Box3()

  // One frame per change, never a standing loop. `pending` collapses several
  // changes in the same tick into a single draw.
  let pending = false
  function requestRender(): void {
    if (pending) return
    pending = true
    requestAnimationFrame(() => {
      pending = false
      renderer.render(scene, camera)
    })
  }
  controls.addEventListener('change', requestRender)

  /** A grabbable handle size: ~1.5% of the skeleton's diagonal, so it works at
   * any creature scale rather than being a speck on a whale or a boulder on a
   * bird. */
  function handleRadius(
    positions: readonly (readonly [number, number, number])[]
  ): number {
    const box = new Box3()
    for (const p of positions) box.expandByPoint(new Vector3(p[0], p[1], p[2]))
    return Math.max(box.getSize(new Vector3()).length() * 0.015, 0.005)
  }

  /** Rewrites the skeleton line from the current edited joint positions. */
  function refreshFittedGeometry(): void {
    if (fittedSkeleton === null) return
    const points = skeletonSegments(editPositions, editParents)
    fittedSkeleton.geometry.setAttribute('position', new BufferAttribute(points, 3))
    fittedSkeleton.geometry.getAttribute('position').needsUpdate = true
    fittedSkeleton.geometry.computeBoundingSphere()
  }

  /** Picks the joint handle under the pointer and attaches the gizmo to it. A
   * miss leaves the current selection alone. */
  function onPointerDown(event: PointerEvent): void {
    if (gizmo === null || gizmo.dragging) return
    const rect = renderer.domElement.getBoundingClientRect()
    pointer.x = ((event.clientX - rect.left) / rect.width) * 2 - 1
    pointer.y = -((event.clientY - rect.top) / rect.height) * 2 + 1
    raycaster.setFromCamera(pointer, camera)
    const hit = raycaster.intersectObjects(jointHandles, false)[0]
    if (hit !== undefined) {
      gizmo.attach(hit.object)
      requestRender()
    }
  }

  /** Tears down any joint handles and the gizmo, restoring the plain overlay. */
  function clearFittedEditing(): void {
    if (gizmo !== null) {
      gizmo.detach()
      scene.remove(gizmo.getHelper())
      renderer.domElement.removeEventListener('pointerdown', onPointerDown)
      gizmo.dispose()
      gizmo = null
    }
    for (const handle of jointHandles) scene.remove(handle)
    jointHandles = []
    handleGeometry?.dispose()
    handleGeometry = null
    handleMaterial?.dispose()
    handleMaterial = null
    onJointEdit = null
    controls.enabled = true
  }

  /** Puts a draggable handle on every joint and a translate gizmo in the scene. */
  function startFittedEditing(
    positions: readonly (readonly [number, number, number])[],
    parents: readonly (number | null)[],
    onEdit: (positions: ReadonlyArray<readonly [number, number, number]>) => void
  ): void {
    editPositions = positions.map((p): [number, number, number] => [p[0], p[1], p[2]])
    editParents = parents
    onJointEdit = onEdit

    handleGeometry = new SphereGeometry(handleRadius(positions), 8, 8)
    // Drawn over the mesh like the skeleton itself, so a handle inside the body
    // is still grabbable.
    handleMaterial = new MeshBasicMaterial({ color: 0xffb454, depthTest: false, transparent: true })
    positions.forEach((p, index) => {
      const handle = new Mesh(handleGeometry as SphereGeometry, handleMaterial as MeshBasicMaterial)
      handle.position.set(p[0], p[1], p[2])
      handle.renderOrder = 2
      handle.userData.jointIndex = index
      jointHandles.push(handle)
      scene.add(handle)
    })

    gizmo = new TransformControls(camera, renderer.domElement)
    gizmo.setMode('translate')
    gizmo.addEventListener('dragging-changed', (event) => {
      const dragging = (event as unknown as { value: boolean }).value
      // Freeing the orbit while dragging would spin the camera with the joint.
      controls.enabled = !dragging
      // Notify once, when the drag ends — one edited placement per drag, so undo
      // steps over whole moves rather than every pixel of one.
      if (!dragging) onJointEdit?.(editPositions)
    })
    gizmo.addEventListener('objectChange', () => {
      const object = gizmo?.object
      if (object === undefined) return
      const index = object.userData.jointIndex as number
      editPositions = withJointMoved(editPositions, index, [
        object.position.x,
        object.position.y,
        object.position.z
      ])
      refreshFittedGeometry()
      requestRender()
    })
    gizmo.addEventListener('change', requestRender)
    scene.add(gizmo.getHelper())
    renderer.domElement.addEventListener('pointerdown', onPointerDown)
  }

  // A frame loop that runs ONLY while a clip plays. The event-driven
  // `requestRender` keeps an idle viewport at zero cost; this is the one place
  // that needs a continuous loop, and it stops itself the moment playback does.
  function tick(): void {
    if (!playing || mixer === null) return
    const now = performance.now()
    mixer.update((now - lastFrame) / 1000)
    lastFrame = now
    renderer.render(scene, camera)
    requestAnimationFrame(tick)
  }

  /** The canvas's own box, not its parent's — the stage also holds the
   * guidance strip, so the parent is taller than the drawing area. */
  function viewportAspect(): number {
    const { clientWidth: width, clientHeight: height } = renderer.domElement
    return width === 0 || height === 0 ? 1 : width / height
  }

  function resize(): void {
    const { clientWidth: width, clientHeight: height } = renderer.domElement
    if (width === 0 || height === 0) return
    // `updateStyle: false` — CSS owns the element's size, this sets only the
    // drawing buffer. Letting the renderer write inline styles would fight the
    // grid and the canvas would grow on every resize.
    renderer.setSize(width, height, false)
    camera.aspect = width / height
    camera.updateProjectionMatrix()
    requestRender()
  }
  new ResizeObserver(resize).observe(renderer.domElement)

  function reframe(): void {
    const framing = frameBounds(bounds, FOV_DEGREES, viewportAspect())
    applyFraming(camera, framing)
    controls.target.copy(framing.target)
    controls.update()
    requestRender()
  }

  /** A material slot holds either one material or an array of them. */
  function disposeMaterial(material: Material | Material[]): void {
    for (const one of Array.isArray(material) ? material : [material]) one.dispose()
  }

  /** Frees the GPU buffers an object tree holds. */
  function release(root: Object3D): void {
    root.traverse((object) => {
      if (!(object instanceof Mesh)) return
      object.geometry.dispose()
      disposeMaterial(object.material)
    })
  }

  return {
    canvas: renderer.domElement,

    async show(data: ArrayBuffer): Promise<ModelContents> {
      const contents = await parseModel(data)

      clearFittedEditing()
      if (model !== null) {
        scene.remove(model)
        release(model)
      }
      if (skeleton !== null) {
        scene.remove(skeleton)
        skeleton.dispose()
        skeleton = null
      }

      model = contents.root
      scene.add(model)
      if (contents.bones > 0) {
        skeleton = new SkeletonHelper(model)
        scene.add(skeleton)
      }

      bounds = contents.bounds
      // The grid is sized to the subject: a 4 m grid under a 30 m creature
      // reads as a postage stamp, and under a 20 cm one as a runway.
      const span = Math.max(bounds.getSize(new Vector3()).length(), 0.1)
      grid.scale.setScalar(span / 2)

      reframe()
      return contents
    },

    async playAnimated(data, clipName): Promise<number | null> {
      const contents = await parseAnimated(data)
      const clip = findClip(contents.clips, clipName)
      if (clip === undefined) return null

      // Replace any previous playback subject.
      this.stop()
      clearFittedEditing()
      if (animated !== null) {
        scene.remove(animated)
        release(animated)
      }
      animated = contents.root
      // The imported mesh and the animated one occupy the same space; show one
      // at a time so they do not z-fight.
      if (model !== null) model.visible = false
      scene.add(animated)

      mixer = new AnimationMixer(animated)
      mixer.clipAction(clip).play()
      playing = true
      lastFrame = performance.now()
      requestAnimationFrame(tick)
      return clip.duration
    },

    async showOverlay(data): Promise<void> {
      const contents = await parseModel(data)
      clearFittedEditing()
      if (overlay !== null) {
        scene.remove(overlay)
        release(overlay)
      }
      overlay = contents.root
      // A material that actually shows the baked vertex colours, lit flat so
      // the hues read true rather than shaded. Unlit keeps the flag colour a
      // flag regardless of where the light is.
      overlay.traverse((object) => {
        if (object instanceof Mesh && hasVertexColors(object)) {
          object.material = new MeshBasicMaterial({ vertexColors: true })
        }
      })
      if (model !== null) model.visible = false
      scene.add(overlay)
      requestRender()
    },

    stop(): void {
      if (overlay !== null) {
        scene.remove(overlay)
        release(overlay)
        overlay = null
        if (model !== null) model.visible = true
      }
      playing = false
      if (mixer !== null) {
        mixer.stopAllAction()
        mixer = null
      }
      if (animated !== null) {
        scene.remove(animated)
        release(animated)
        animated = null
      }
      if (model !== null) model.visible = true
      requestRender()
    },

    showFittedSkeleton(positions, parents, onEdit): void {
      clearFittedEditing()
      if (fittedSkeleton !== null) {
        scene.remove(fittedSkeleton)
        fittedSkeleton.geometry.dispose()
        disposeMaterial(fittedSkeleton.material)
        fittedSkeleton = null
      }

      const points = skeletonSegments(positions, parents)
      if (points.length === 0) {
        requestRender()
        return
      }

      const geometry = new BufferGeometry()
      geometry.setAttribute('position', new BufferAttribute(points, 3))
      fittedSkeleton = new LineSegments(
        geometry,
        // Drawn over the mesh rather than through it: a skeleton the body hides
        // is a skeleton nobody can check.
        new LineBasicMaterial({ color: 0xffb454, depthTest: false, transparent: true })
      )
      fittedSkeleton.renderOrder = 1
      scene.add(fittedSkeleton)
      if (onEdit !== undefined) startFittedEditing(positions, parents, onEdit)
      requestRender()
    },

    reframe,

    dispose(): void {
      playing = false
      clearFittedEditing()
      if (overlay !== null) release(overlay)
      if (animated !== null) release(animated)
      if (model !== null) release(model)
      if (fittedSkeleton !== null) {
        fittedSkeleton.geometry.dispose()
        disposeMaterial(fittedSkeleton.material)
      }
      skeleton?.dispose()
      controls.dispose()
      renderer.dispose()
    }
  }
}
