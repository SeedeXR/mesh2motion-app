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
  type Object3D,
  PerspectiveCamera,
  Scene,
  SkeletonHelper,
  Vector3,
  WebGLRenderer
} from 'three'
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js'
import {
  applyFraming,
  findClip,
  frameBounds,
  parseAnimated,
  parseModel,
  skeletonSegments,
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
    parents: readonly (number | null)[]
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

  // Playback state. The animated model is a separate object from `model` — the
  // imported mesh — so playing a clip does not disturb what the earlier steps
  // put on screen; it is removed again on `stop`.
  let animated: Object3D | null = null
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

    stop(): void {
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

    showFittedSkeleton(positions, parents): void {
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
      requestRender()
    },

    reframe,

    dispose(): void {
      playing = false
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
