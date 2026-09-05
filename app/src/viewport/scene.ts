/**
 * The Three.js viewport.
 *
 * Everything decidable without a GPU lives in `model.ts`; this is the part that
 * needs a screen, and it is kept thin on purpose because it is the part no test
 * can look at.
 *
 * # Rendering runs a standing loop
 *
 * `setAnimationLoop` draws every frame, the way a 3D tool's viewport does. A
 * draw-on-change renderer would save idle CPU (todo P4-5), but the always-on
 * loop is simpler and can never miss a state change, and it is where the
 * animation mixer is advanced while a clip plays. Orbit damping stays off:
 * it buys nothing and only adds work now that a frame is drawn regardless.
 */

import {
  type AnimationAction,
  AmbientLight,
  AnimationMixer,
  Box3,
  BufferAttribute,
  BufferGeometry,
  Color,
  DirectionalLight,
  DoubleSide,
  type Material,
  Mesh,
  MeshBasicMaterial,
  MeshStandardMaterial,
  MOUSE,
  NeutralToneMapping,
  type Object3D,
  PerspectiveCamera,
  PlaneGeometry,
  CanvasTexture,
  PMREMGenerator,
  Raycaster,
  Scene,
  SRGBColorSpace,
  ShaderMaterial,
  Spherical,
  IcosahedronGeometry,
  Sprite,
  SpriteMaterial,
  TOUCH,
  Vector2,
  Vector3,
  WebGLRenderer
} from 'three'
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js'
import { TransformControls } from 'three/examples/jsm/controls/TransformControls.js'
import { RoomEnvironment } from 'three/examples/jsm/environments/RoomEnvironment.js'
import {
  applyFraming,
  findClip,
  frameBounds,
  hasVertexColors,
  localHandleRadius,
  parseAnimated,
  parseModel,
  presetCameraPosition,
  skeletonOctahedra,
  withJointMoved,
  type ModelContents,
  type ViewPreset
} from './model'

const FOV_DEGREES = 45

// The marker-placement "precision preview": a magnified inset (Mixamo's loupe)
// pinned to the viewport's top-right, showing a close-up of the joint under the
// pointer so a marker can be dropped exactly. It sits just below the persistent
// viewport-nav toolbar (LOUPE_TOP clears it). Fixed pixel size — this flow is a
// desktop interaction. ponytail: make it responsive if it ever ships on small
// screens.
const LOUPE_SIZE = 180
const LOUPE_MARGIN = 12
const LOUPE_TOP = 60
const LOUPE_ZOOM = 9

/**
 * Rewrites OrbitControls' button map on each pointer-down to mimic Blender.
 *
 * Blender navigates with the middle mouse button: MMB orbits, Shift+MMB pans,
 * Ctrl+MMB zooms. A Magic Mouse or trackpad has no middle button, so — as
 * Blender's own "emulate 3-button mouse" does — Option(Alt)+left stands in for
 * it. Plain left is left unbound for selection (Blender's LMB-select). The
 * capture phase runs this before OrbitControls reads `mouseButtons`.
 */
function installBlenderNavigation(controls: OrbitControls, dom: HTMLElement): void {
  const action = (e: PointerEvent): MOUSE => {
    if (e.shiftKey) return MOUSE.PAN
    if (e.ctrlKey || e.metaKey) return MOUSE.DOLLY
    return MOUSE.ROTATE
  }
  dom.addEventListener(
    'pointerdown',
    (e) => {
      if (e.button === 1) {
        controls.mouseButtons = { LEFT: undefined, MIDDLE: action(e), RIGHT: MOUSE.PAN }
      } else if (e.button === 0 && e.altKey) {
        controls.mouseButtons = { LEFT: action(e), MIDDLE: MOUSE.ROTATE, RIGHT: MOUSE.PAN }
      } else if (e.button === 0) {
        controls.mouseButtons = { LEFT: undefined, MIDDLE: MOUSE.ROTATE, RIGHT: MOUSE.PAN }
      }
    },
    true
  )
}

/** A ground grid that reads as infinite, the way Blender's does. */
interface InfiniteGrid {
  readonly mesh: Mesh
  /** Sizes the cells and the fade to the subject's span (metres). */
  setScale(span: number): void
  /** Re-centres the grid under the camera and updates the distance fade. */
  update(camera: PerspectiveCamera): void
  dispose(): void
}

/**
 * An infinite ground grid: a large plane kept under the camera whose shader
 * draws world-anchored lines and fades them out with distance, so there is no
 * visible edge and the grid appears to go on forever. Minor lines every cell,
 * brighter lines every ten. `fwidth` keeps every line one pixel wide at any
 * zoom (WebGL2, which three uses, has derivatives built in).
 */
function makeInfiniteGrid(): InfiniteGrid {
  const geometry = new PlaneGeometry(1, 1)
  geometry.rotateX(-Math.PI / 2)
  const uniforms = {
    uCell: { value: 0.5 },
    uMinor: { value: new Color(0x2a2f37) },
    uMajor: { value: new Color(0x3b4351) },
    uCamPos: { value: new Vector3() },
    uFade: { value: 40 }
  }
  const material = new ShaderMaterial({
    uniforms,
    transparent: true,
    depthWrite: false,
    side: DoubleSide,
    vertexShader: `
      varying vec3 vWorld;
      void main() {
        vWorld = (modelMatrix * vec4(position, 1.0)).xyz;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }`,
    fragmentShader: `
      precision highp float;
      varying vec3 vWorld;
      uniform float uCell;
      uniform vec3 uMinor;
      uniform vec3 uMajor;
      uniform vec3 uCamPos;
      uniform float uFade;
      float grid(vec2 coord, float cell) {
        vec2 g = abs(fract(coord / cell - 0.5) - 0.5) / fwidth(coord / cell);
        return 1.0 - min(min(g.x, g.y), 1.0);
      }
      void main() {
        vec2 c = vWorld.xz;
        float minor = grid(c, uCell);
        float major = grid(c, uCell * 10.0);
        float fade = 1.0 - clamp(distance(c, uCamPos.xz) / uFade, 0.0, 1.0);
        float a = max(minor * 0.5, major) * fade;
        if (a < 0.002) discard;
        gl_FragColor = vec4(mix(uMinor, uMajor, clamp(major, 0.0, 1.0)), a);
      }`
  })
  const mesh = new Mesh(geometry, material)
  mesh.frustumCulled = false
  mesh.renderOrder = -1
  return {
    mesh,
    setScale(span) {
      // A power-of-ten cell near a tenth of the subject, so the grid reads the
      // same under a 20 cm bird and a 30 m whale. The fade and the plane are
      // sized to it, the plane always wider than the fade so its edge never shows.
      const cell = Math.pow(10, Math.round(Math.log10(Math.max(span, 0.01) / 10)))
      uniforms.uCell.value = cell
      uniforms.uFade.value = Math.max(span * 20, cell * 40)
      mesh.scale.setScalar(uniforms.uFade.value * 2.2)
    },
    update(camera) {
      mesh.position.set(camera.position.x, 0, camera.position.z)
      uniforms.uCamPos.value.copy(camera.position)
    },
    dispose() {
      geometry.dispose()
      material.dispose()
    }
  }
}

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
  /** Pauses or resumes the running clip without tearing it down. */
  setPaused(paused: boolean): void
  /** Plays the clip forward (`1`) or in reverse (`-1`). */
  setPlaybackDirection(direction: 1 | -1): void
  /** Jumps the clip to `seconds` and shows that pose. */
  seek(seconds: number): void
  /** The clip's current playback time in seconds (0 when nothing plays). */
  playbackTime(): number
  /** Stops playback and returns to the event-driven, zero-idle-cost renderer. */
  stop(): void
  /** Shows or hides the fitted-skeleton bones (the Animate step's hide-bones
   *  toggle). Honoured across play/stop until changed. */
  setSkeletonVisible(visible: boolean): void
  /**
   * Draws a weight-paint overlay: a colour-baked model shown with its vertex
   * colours, replacing whatever was on screen. Returns to `showFittedSkeleton`
   * territory when the user leaves; call `show` again to restore the plain mesh.
   */
  showOverlay(data: ArrayBuffer): Promise<void>
  /** Frames the current model again, e.g. after the panel layout changed. */
  reframe(): void
  /** Snaps the camera to a canonical view (front/back/left/right/top/bottom),
   *  keeping the current distance and target. */
  setView(preset: ViewPreset): void
  /** Dollies the camera toward (`factor` < 1) or away from (`factor` > 1) the
   *  target — the on-screen zoom buttons. */
  zoom(factor: number): void
  /**
   * Puts a rotate or translate gizmo on the model so the user can reorient or
   * reposition it independent of the grid — for a mesh that imported lying down
   * or off-centre. `'none'` removes the gizmo.
   */
  setTransformMode(mode: 'none' | 'rotate' | 'translate'): void
  /** Removes the fitted-skeleton overlay and its edit handles, leaving the mesh. */
  clearFittedSkeleton(): void
  /**
   * Enters marker-placement mode. A left-click on the model drops the active
   * marker via `onPlace`; grabbing an existing marker and dragging moves it via
   * `onMove` (its slot id, the new surface point). `endMarkerPlacement` leaves
   * it. Picks that miss the mesh are ignored.
   */
  beginMarkerPlacement(handlers: {
    onPlace: (point: [number, number, number]) => void
    onMove: (id: string, point: [number, number, number]) => void
  }): void
  /** Draws the given markers as camera-facing rings, replacing any shown. */
  setMarkers(
    markers: readonly { id: string; position: readonly [number, number, number]; color: number }[]
  ): void
  /** Leaves marker-placement mode and removes every marker sphere. */
  endMarkerPlacement(): void
  /** The model's symmetry plane on X (its bounds centre), for mirroring markers. */
  symmetryX(): number
  /** A one-line diagnostic string: canvas size, buffer size, frames drawn, and
   *  the active render backend. For "the viewport is blank" reports. */
  info(): string
  /** Releases the GPU resources this viewport holds. */
  dispose(): void
}

/** Creates a viewport. The canvas is returned unmounted. */
export function createViewport(): Viewport {
  const renderer = new WebGLRenderer({ antialias: true })
  // Above 2x the cost grows faster than anyone can see the difference, and a
  // 3x display would otherwise render nine times the pixels.
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2))
  // Colour fidelity, so a textured model looks the way it does in Blender or
  // Maya rather than dull and flat. The KHR PBR-Neutral tone map preserves hue
  // and saturation while taming highlights (unlike ACES, which shifts colour);
  // the output is sRGB, which is three's default but pinned here so it cannot
  // drift.
  renderer.toneMapping = NeutralToneMapping
  renderer.toneMappingExposure = 1.0
  renderer.outputColorSpace = SRGBColorSpace

  // Frames-drawn counter for the "viewport is blank" diagnostic (`info()`).
  let renders = 0

  const scene = new Scene()
  scene.background = new Color(0x14161a)

  const camera = new PerspectiveCamera(FOV_DEGREES, 1, 0.01, 100)
  // The precision-preview loupe renders the scene a second time, from the same
  // angle but pulled in toward the marked joint, into a square inset.
  const loupeCamera = new PerspectiveCamera(FOV_DEGREES, 1, 0.01, 100)
  let loupeActive = false
  let loupeShown = false
  const loupeTarget = new Vector3()
  const loupeDir = new Vector3()
  const loupeSize = new Vector2()
  let loupeEl: HTMLDivElement | null = null
  const controls = new OrbitControls(camera, renderer.domElement)
  // Damping gives the gentle glide a trackpad flick expects (the standing loop
  // draws every frame anyway, so its follow-up frames cost nothing). Zooming to
  // the cursor, not the centre, is what makes a Magic Mouse's small scroll
  // deltas feel precise. Screen-space panning keeps a pan parallel to the
  // screen regardless of camera pitch — moving the view independent of the grid.
  controls.enableDamping = true
  controls.dampingFactor = 0.08
  controls.zoomToCursor = true
  controls.screenSpacePanning = true
  controls.rotateSpeed = 0.85
  controls.zoomSpeed = 1.1
  controls.panSpeed = 0.85
  // Blender-style navigation for familiarity: the MIDDLE mouse button orbits,
  // Shift+MMB pans, Ctrl+MMB zooms. A Magic Mouse / trackpad has no middle
  // button, so Option(Alt)+LEFT stands in for it — Blender's own "emulate
  // 3-button mouse". Plain LEFT is left unbound so it can select joint handles,
  // exactly as LMB selects in Blender. `blenderNavigation` rewrites these per
  // pointer-down from the held modifiers just before OrbitControls reads them.
  controls.mouseButtons = { LEFT: undefined, MIDDLE: MOUSE.ROTATE, RIGHT: MOUSE.PAN }
  controls.touches = { ONE: TOUCH.ROTATE, TWO: TOUCH.DOLLY_PAN }
  installBlenderNavigation(controls, renderer.domElement)

  // Image-based lighting from a neutral studio room — the same trick the glTF
  // sample viewer and three's editor use. It gives PBR materials soft ambient
  // and gentle reflections, which is what makes a model read as lit rather than
  // flat; a single directional key still shapes the form. Generated once via
  // PMREM and kept as the scene's environment.
  const pmrem = new PMREMGenerator(renderer)
  scene.environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture
  scene.add(new AmbientLight(0xffffff, 0.35))
  const key = new DirectionalLight(0xffffff, 1.4)
  key.position.set(2, 4, 3)
  scene.add(key)

  const grid = makeInfiniteGrid()
  scene.add(grid.mesh)

  let model: Object3D | null = null
  // The imported model's own rig, drawn as octahedral bones (like Blender and
  // like the fitted skeleton) rather than three's thin SkeletonHelper lines. A
  // child of the model, so it follows the orient gizmo without any extra work.
  let importedSkeleton: Mesh | null = null
  let fittedSkeleton: Mesh | null = null
  // Whether the fitted-skeleton bones may show. The Animate step turns this off
  // (Mixamo shows just the moving mesh); it's honoured across play/stop so bones
  // don't pop back when a clip ends.
  let skeletonVisible = true
  // The octahedral bones' material: solid, flat-shaded so each facet catches the
  // light and the bone reads as 3D, drawn over the mesh (depthTest off) so a
  // buried bone stays visible. DoubleSide sidesteps any face-winding surprise.
  const boneMaterial = new MeshBasicMaterial({
    // Mixamo-style skeleton: a per-vertex blue→violet hierarchy-depth gradient
    // (set in skeletonOctahedra) rather than flat Blender grey, so the skeleton
    // reads as the primary rig. Unlit + `toneMapped:false` (the weight overlay's
    // trick) so the bright studio IBL cannot wash the saturated hues to white —
    // which is exactly what MeshStandard did here.
    vertexColors: true,
    toneMapped: false,
    depthTest: false,
    transparent: true,
    opacity: 0.95,
    side: DoubleSide
  })

  // Bone-placement editing (Fit step). When `showFittedSkeleton` is given an
  // edit callback, a draggable handle sits on each joint and a translate gizmo
  // attaches to whichever the user clicks. The edited positions feed straight
  // back to binding — the callback is how they get there.
  let jointHandles: Mesh[] = []
  let handleGeometry: IcosahedronGeometry | null = null
  // Three shared handle materials so a joint can signal its state by colour, in
  // Blender's grey key: light grey at rest (a distinct grabbable knob against
  // the grey bones), white on hover (the cursor is over it, click to grab),
  // amber when selected (the gizmo is on it and arrow-keys nudge it — one warm
  // highlight so the active joint is unmistakable). Drawn over the mesh
  // (depthTest off) so a buried joint stays visible and grabbable.
  // Flat-shaded and lit, so the low-poly facets (see handleGeometry) catch the
  // light and each handle reads as a solid 3D knob you can orient, not a flat
  // disc — which an unlit MeshBasic sphere looks like.
  const handleMat = (color: number): MeshStandardMaterial =>
    new MeshStandardMaterial({
      color,
      flatShading: true,
      roughness: 0.5,
      metalness: 0.0,
      depthTest: false,
      transparent: true
    })
  const handleRest = handleMat(0xd0d4da)
  const handleHover = handleMat(0xf2f4f6)
  const handleSelected = handleMat(0xffb454)
  let hoveredHandle: Mesh | null = null
  let selectedHandle: Mesh | null = null
  // Arrow-key nudge distance (one press), sized to the skeleton so it is a fine
  // touch at any creature scale; `nudgeDirty` batches a burst of presses into a
  // single undo step, committed on key-up.
  let nudgeStep = 0.01
  let nudgeDirty = false
  let gizmo: TransformControls | null = null
  let editPositions: Array<[number, number, number]> = []
  let editParents: readonly (number | null)[] = []
  let onJointEdit: ((positions: ReadonlyArray<readonly [number, number, number]>) => void) | null =
    null
  const raycaster = new Raycaster()
  const pointer = new Vector2()

  // Model-placement gizmo (rotate / move the whole model, not a joint). Attaches
  // to `model` so the mesh and its skeleton reorient together, independent of
  // the grid. Separate from the joint gizmo above, which edits one bone.
  let modelGizmo: TransformControls | null = null

  // Playback state. The animated model is a separate object from `model` — the
  // imported mesh — so playing a clip does not disturb what the earlier steps
  // put on screen; it is removed again on `stop`.
  let animated: Object3D | null = null
  let overlay: Object3D | null = null
  let mixer: AnimationMixer | null = null
  // The clip's action, kept so the transport (pause, direction, scrub) can drive
  // it. `paused` freezes the mixer at its current time without tearing playback
  // down, so a scrub still shows the right pose.
  let action: AnimationAction | null = null
  let paused = false
  // Elapsed-time tracked with performance.now() rather than three's Clock,
  // which is deprecated. `lastFrame` is the timestamp the mixer last advanced
  // from; the delta between frames is what it needs.
  let lastFrame = 0
  let playing = false
  let bounds = new Box3()

  // A standing render loop drives every frame and advances the animation mixer
  // while a clip plays (unless the transport has paused it).
  renderer.setAnimationLoop((now: number) => {
    if (playing && !paused && mixer !== null) mixer.update((now - lastFrame) / 1000)
    lastFrame = now
    controls.update()
    grid.update(camera)
    renderer.render(scene, camera)
    if (loupeActive && loupeShown) renderLoupe()
    renders++
  })


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


  /** The floor on octahedral-bone width — a slim ~28% of a joint handle, so the
   * grabbable spheres still read as the pick targets and short bones (fingers,
   * toes) stay slender rather than bloating to the minimum. Long bones size off
   * their own length (see `skeletonOctahedra`). */
  function boneWidth(positions: readonly (readonly [number, number, number])[]): number {
    return handleRadius(positions) * 0.28
  }

  /** Fills `geometry` with octahedral bones for the given joints, with normals
   * for the flat shading. Shared by the initial draw and every edit refresh. */
  function fillBoneGeometry(
    geometry: BufferGeometry,
    positions: readonly (readonly [number, number, number])[],
    parents: readonly (number | null)[]
  ): void {
    const { positions: verts, indices, colors } = skeletonOctahedra(
      positions,
      parents,
      boneWidth(positions)
    )
    geometry.setIndex(new BufferAttribute(indices, 1))
    geometry.setAttribute('position', new BufferAttribute(verts, 3))
    geometry.setAttribute('color', new BufferAttribute(colors, 3))
    geometry.computeVertexNormals()
    geometry.computeBoundingSphere()
  }

  /** Rebuilds the octahedral bones from the current edited joint positions. */
  function refreshFittedGeometry(): void {
    if (fittedSkeleton === null) return
    fillBoneGeometry(fittedSkeleton.geometry, editPositions, editParents)
  }

  /**
   * The imported model's rig as octahedral bones, in the model's own local
   * space so the mesh drawn as its child follows the model's transform.
   *
   * Reads the first skinned mesh's skeleton by three's `isSkinnedMesh` / `isBone`
   * flags, so it needs no extra class imports. Returns null when the model has no
   * skinned skeleton or too few bones to draw.
   */
  function importedBoneMesh(root: Object3D): Mesh | null {
    root.updateWorldMatrix(true, true)
    let bones: Object3D[] | null = null
    root.traverse((object) => {
      const skinned = object as { isSkinnedMesh?: boolean; skeleton?: { bones: Object3D[] } }
      if (bones === null && skinned.isSkinnedMesh === true && skinned.skeleton !== undefined) {
        bones = skinned.skeleton.bones
      }
    })
    if (bones === null) return null
    const list: Object3D[] = bones
    if (list.length < 2) return null

    const index = new Map<Object3D, number>()
    list.forEach((bone, i) => index.set(bone, i))
    const positions = list.map((bone) => {
      const p = root.worldToLocal(bone.getWorldPosition(new Vector3()))
      return [p.x, p.y, p.z] as [number, number, number]
    })
    const parents = list.map((bone) => {
      const parent = bone.parent
      return parent !== null && index.has(parent) ? (index.get(parent) as number) : null
    })

    const geometry = new BufferGeometry()
    fillBoneGeometry(geometry, positions, parents)
    if (geometry.getIndex()?.count === 0) {
      geometry.dispose()
      return null
    }
    // Occluded by the mesh, the way Blender draws an armature by default — so a
    // textured character reads clean, not covered in bones. (The fitted skeleton,
    // which is for editing, still draws on top.)
    const material = boneMaterial.clone()
    material.depthTest = true
    material.transparent = false
    material.opacity = 1
    const mesh = new Mesh(geometry, material)
    // Not a pick target: marker and joint raycasts must reach the mesh, not this.
    mesh.raycast = () => {}
    return mesh
  }

  /** Paints a handle for a state and sizes it to match: rest is its base size,
   * hover and select grow from it so the active joint is an obviously bigger,
   * easier grab target — the Blender-like precise-fit feel, a touch friendlier. */
  function paintHandle(handle: Mesh, state: 'rest' | 'hover' | 'selected'): void {
    const base = handle.userData.baseScale as number
    if (state === 'selected') {
      handle.material = handleSelected
      handle.scale.setScalar(base * 1.4)
    } else if (state === 'hover') {
      handle.material = handleHover
      handle.scale.setScalar(base * 1.25)
    } else {
      handle.material = handleRest
      handle.scale.setScalar(base)
    }
  }

  /** The joint handle under the pointer, or null. Shared by hover and picking. */
  function handleUnderPointer(event: PointerEvent): Mesh | null {
    const rect = renderer.domElement.getBoundingClientRect()
    pointer.x = ((event.clientX - rect.left) / rect.width) * 2 - 1
    pointer.y = -((event.clientY - rect.top) / rect.height) * 2 + 1
    raycaster.setFromCamera(pointer, camera)
    const hit = raycaster.intersectObjects(jointHandles, false)[0]
    return (hit?.object as Mesh | undefined) ?? null
  }

  /** Picks the joint handle under the pointer and attaches the gizmo to it. A
   * miss leaves the current selection alone. */
  function onPointerDown(event: PointerEvent): void {
    if (gizmo === null || gizmo.dragging) return
    const handle = handleUnderPointer(event)
    if (handle !== null) {
      if (selectedHandle !== null && selectedHandle !== handle) paintHandle(selectedHandle, 'rest')
      selectedHandle = handle
      paintHandle(handle, 'selected')
      gizmo.attach(handle)
    }
  }

  /** Highlights the handle under the cursor and shows a grab cursor over it, so
   * it is obvious what a click will pick before the click happens. */
  function onPointerMove(event: PointerEvent): void {
    if (gizmo?.dragging === true) return
    const handle = handleUnderPointer(event)
    if (handle === hoveredHandle) return
    if (hoveredHandle !== null && hoveredHandle !== selectedHandle) paintHandle(hoveredHandle, 'rest')
    hoveredHandle = handle
    if (handle !== null && handle !== selectedHandle) paintHandle(handle, 'hover')
    renderer.domElement.style.cursor = handle !== null ? 'pointer' : ''
  }

  /** Arrow keys nudge the selected joint in the screen plane for a precise final
   * placement; Shift makes the step five times finer. The move is batched — one
   * undo step per burst — and committed on key-up. */
  function onEditKeyDown(event: KeyboardEvent): void {
    if (selectedHandle === null) return
    const axes: Readonly<Record<string, readonly [number, number]>> = {
      ArrowLeft: [-1, 0],
      ArrowRight: [1, 0],
      ArrowUp: [0, 1],
      ArrowDown: [0, -1]
    }
    const axis = axes[event.key]
    if (axis === undefined) return
    event.preventDefault()
    const step = event.shiftKey ? nudgeStep / 5 : nudgeStep
    // Screen-space: move along the camera's right and up axes so a nudge tracks
    // what the user sees, not world axes they'd have to reason about.
    const right = new Vector3().setFromMatrixColumn(camera.matrixWorld, 0)
    const up = new Vector3().setFromMatrixColumn(camera.matrixWorld, 1)
    const delta = right.multiplyScalar(axis[0] * step).add(up.multiplyScalar(axis[1] * step))
    selectedHandle.position.add(delta)
    const index = selectedHandle.userData.jointIndex as number
    editPositions = withJointMoved(editPositions, index, [
      selectedHandle.position.x,
      selectedHandle.position.y,
      selectedHandle.position.z
    ])
    refreshFittedGeometry()
    nudgeDirty = true
  }

  /** Commits a nudge burst once the keys are released, as a single edit. */
  function onEditKeyUp(): void {
    if (!nudgeDirty) return
    nudgeDirty = false
    onJointEdit?.(editPositions)
  }

  /** Tears down any joint handles and the gizmo, restoring the plain overlay. */
  function clearFittedEditing(): void {
    if (gizmo !== null) {
      gizmo.detach()
      scene.remove(gizmo.getHelper())
      renderer.domElement.removeEventListener('pointerdown', onPointerDown)
      renderer.domElement.removeEventListener('pointermove', onPointerMove)
      window.removeEventListener('keydown', onEditKeyDown)
      window.removeEventListener('keyup', onEditKeyUp)
      gizmo.dispose()
      gizmo = null
    }
    for (const handle of jointHandles) scene.remove(handle)
    jointHandles = []
    handleGeometry?.dispose()
    handleGeometry = null
    hoveredHandle = null
    selectedHandle = null
    nudgeDirty = false
    renderer.domElement.style.cursor = ''
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

    const maxRadius = handleRadius(positions)
    // A unit low-poly sphere scaled per joint, so each handle is sized to its
    // local joint spacing (see localHandleRadius) — the finger handles no longer
    // merge into one unpickable clump. An icosahedron (detail 1, ~80 facets) with
    // flat shading reads as a solid, orientable 3D knob rather than a flat disc,
    // which makes dragging it in x/y/z easier to judge.
    handleGeometry = new IcosahedronGeometry(1, 1)
    // A nudge of ~15% of the full handle radius is a fine touch that still moves
    // visibly; Shift makes it finer still for the dense joints.
    nudgeStep = maxRadius * 0.15
    positions.forEach((p, index) => {
      const handle = new Mesh(handleGeometry as IcosahedronGeometry, handleRest)
      handle.position.set(p[0], p[1], p[2])
      const base = localHandleRadius(positions, index, maxRadius)
      // The resting size; hover and select grow from it (see paintHandle) so the
      // joint under the cursor is a bigger, easier target as you reach for it.
      handle.userData.baseScale = base
      handle.scale.setScalar(base)
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
    })
    scene.add(gizmo.getHelper())
    renderer.domElement.addEventListener('pointerdown', onPointerDown)
    renderer.domElement.addEventListener('pointermove', onPointerMove)
    window.addEventListener('keydown', onEditKeyDown)
    window.addEventListener('keyup', onEditKeyUp)
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
  }
  new ResizeObserver(resize).observe(renderer.domElement)

  function reframe(): void {
    const framing = frameBounds(bounds, FOV_DEGREES, viewportAspect())
    applyFraming(camera, framing)
    controls.target.copy(framing.target)
    controls.update()
  }

  function setView(preset: ViewPreset): void {
    const distance = camera.position.distanceTo(controls.target)
    camera.position.copy(presetCameraPosition(preset, controls.target, distance))
    // A top/bottom view looks straight down the up-axis, where azimuth is
    // undefined; roll `up` onto Z for those so the framing has a defined roll,
    // and keep the world up (+Y) for the side views.
    let upZ = 0
    if (preset === 'top') upZ = -1
    else if (preset === 'bottom') upZ = 1
    camera.up.set(0, upZ === 0 ? 1 : 0, upZ)
    camera.lookAt(controls.target)
    controls.update()
  }

  function zoom(factor: number): void {
    // Move the camera along its line to the target; clamp so a runaway click
    // can't cross the target or fly past the far plane.
    const toCamera = camera.position.clone().sub(controls.target)
    const distance = Math.min(Math.max(toCamera.length() * factor, 0.01), 1000)
    camera.position.copy(controls.target).add(toCamera.setLength(distance))
    controls.update()
  }

  function setTransformMode(mode: 'none' | 'rotate' | 'translate'): void {
    if (mode === 'none' || model === null) {
      if (modelGizmo !== null) {
        modelGizmo.detach()
        scene.remove(modelGizmo.getHelper())
        modelGizmo.dispose()
        modelGizmo = null
      }
      return
    }
    if (modelGizmo === null) {
      modelGizmo = new TransformControls(camera, renderer.domElement)
      modelGizmo.addEventListener('dragging-changed', (event) => {
        controls.enabled = !(event as unknown as { value: boolean }).value
      })
      scene.add(modelGizmo.getHelper())
    }
    modelGizmo.setMode(mode)
    modelGizmo.attach(model)
  }

  /** Orbits the camera around the target by a screen-space delta — the trackpad
   *  two-finger swipe, which the browser delivers as a wheel event. */
  function trackpadOrbit(dx: number, dy: number): void {
    const offset = camera.position.clone().sub(controls.target)
    const spherical = new Spherical().setFromVector3(offset)
    spherical.theta -= dx * 0.005
    spherical.phi = Math.max(0.001, Math.min(Math.PI - 0.001, spherical.phi - dy * 0.005))
    // Turntable orbit keeps the world up, like Blender's default.
    camera.up.set(0, 1, 0)
    camera.position.copy(controls.target).add(offset.setFromSpherical(spherical))
    controls.update()
  }

  /** Pans the camera and target together by a screen-space delta — Shift with
   *  the trackpad two-finger swipe. */
  function trackpadPan(dx: number, dy: number): void {
    const scale = camera.position.distanceTo(controls.target) * 0.002
    const right = new Vector3().setFromMatrixColumn(camera.matrixWorld, 0)
    const up = new Vector3().setFromMatrixColumn(camera.matrixWorld, 1)
    const move = right.multiplyScalar(-dx * scale).add(up.multiplyScalar(dy * scale))
    camera.position.add(move)
    controls.target.add(move)
    controls.update()
  }

  /**
   * All wheel gestures, mapped Blender-style. A Mac trackpad delivers a
   * two-finger swipe as a plain wheel event and a pinch as a wheel event with
   * `ctrlKey`; a mouse wheel is told apart by the WebKit/Blink signature
   * `wheelDeltaY === -3 * deltaY`, which only a trackpad produces. So: pinch and
   * mouse wheel zoom, a two-finger swipe orbits (Shift pans) — the swipe that
   * used to zoom now rotates the view, model and grid together, as in Blender.
   * Runs in the capture phase and stops the event so OrbitControls' own wheel
   * zoom never also fires.
   */
  function onWheel(e: WheelEvent): void {
    if (e.target !== renderer.domElement) return
    e.preventDefault()
    e.stopPropagation()
    // A pinch arrives as a wheel with ctrlKey (the browser's convention); Blender
    // also zooms on Ctrl+two-finger, which on a Mac is Ctrl or ⌘. All zoom.
    if (e.ctrlKey || e.metaKey) {
      zoom(1 + e.deltaY * 0.01)
      return
    }
    const legacy = e as unknown as { wheelDeltaY?: number }
    const trackpad =
      legacy.wheelDeltaY !== undefined ? legacy.wheelDeltaY === -3 * e.deltaY : e.deltaMode === 0
    if (!trackpad) {
      zoom(e.deltaY > 0 ? 1.1 : 0.9)
    } else if (e.shiftKey) {
      trackpadPan(e.deltaX, e.deltaY)
    } else {
      trackpadOrbit(e.deltaX, e.deltaY)
    }
  }
  window.addEventListener('wheel', onWheel, { capture: true, passive: false })

  // Blender numpad shortcuts: 1/3/7 snap to front/right/top (Ctrl for the
  // opposite), 4/6/8/2 orbit the view in 15° steps, and Decimal frames the
  // model. Ignored while typing in a field.
  const NUMPAD_VIEWS: Readonly<Record<string, readonly [ViewPreset, ViewPreset]>> = {
    Numpad1: ['front', 'back'],
    Numpad3: ['right', 'left'],
    Numpad7: ['top', 'bottom']
  }
  // 15° expressed in `trackpadOrbit`'s screen-delta units (which scale by 0.005).
  const STEP = Math.PI / 12 / 0.005
  const NUMPAD_ORBITS: Readonly<Record<string, readonly [number, number]>> = {
    Numpad4: [STEP, 0],
    Numpad6: [-STEP, 0],
    Numpad8: [0, STEP],
    Numpad2: [0, -STEP]
  }
  function onNumpadView(event: KeyboardEvent): void {
    const target = event.target
    if (target instanceof HTMLElement && /^(INPUT|TEXTAREA)$/.test(target.tagName)) return
    const pair = NUMPAD_VIEWS[event.code]
    const orbit = NUMPAD_ORBITS[event.code]
    if (pair !== undefined) {
      event.preventDefault()
      setView(pair[event.ctrlKey || event.metaKey ? 1 : 0])
    } else if (orbit !== undefined) {
      event.preventDefault()
      trackpadOrbit(orbit[0], orbit[1])
    } else if (event.code === 'NumpadDecimal') {
      event.preventDefault()
      reframe()
    }
  }
  window.addEventListener('keydown', onNumpadView)

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

  // --- Marker placement (Fit step, marker-rig flow) ------------------------
  // A person drops chin/wrist/elbow/knee/groin markers on the model; each click
  // raycasts the mesh for a surface point and reports it. The app owns the slot
  // and symmetry logic and hands back the markers to draw.
  //
  // Markers are camera-facing sprites of a hollow ring with a centre dot — the
  // Mixamo auto-rigger look — drawn on top of the mesh so they never hide.
  let markerSprites: Sprite[] = []
  const ringTextures = new Map<number, CanvasTexture>()
  /** Placing a new marker, or moving an existing one by its slot id. */
  interface MarkerHandlers {
    onPlace: (point: [number, number, number]) => void
    onMove: (id: string, point: [number, number, number]) => void
  }
  let markerHandlers: MarkerHandlers | null = null
  /** The slot id of the marker being dragged, or null when not dragging. */
  let draggingMarker: string | null = null

  /** A cached hollow-ring-plus-centre-dot texture in the marker's colour. */
  function ringTexture(color: number): CanvasTexture {
    const cached = ringTextures.get(color)
    if (cached !== undefined) return cached
    const size = 128
    const canvas = document.createElement('canvas')
    canvas.width = size
    canvas.height = size
    const ctx = canvas.getContext('2d')
    const texture = new CanvasTexture(canvas)
    texture.colorSpace = SRGBColorSpace
    ringTextures.set(color, texture)
    if (ctx === null) return texture // jsdom / no 2d context: an empty sprite.
    const hex = `#${color.toString(16).padStart(6, '0')}`
    const c = size / 2
    ctx.strokeStyle = hex
    ctx.lineWidth = size * 0.1
    ctx.beginPath()
    ctx.arc(c, c, size * 0.36, 0, Math.PI * 2)
    ctx.stroke()
    ctx.fillStyle = hex
    ctx.beginPath()
    ctx.arc(c, c, size * 0.09, 0, Math.PI * 2)
    ctx.fill()
    texture.needsUpdate = true
    return texture
  }

  /** The surface point under the pointer on the model, or null off the mesh. */
  function markerSurfacePoint(event: PointerEvent): [number, number, number] | null {
    if (model === null) return null
    const rect = renderer.domElement.getBoundingClientRect()
    pointer.x = ((event.clientX - rect.left) / rect.width) * 2 - 1
    pointer.y = -((event.clientY - rect.top) / rect.height) * 2 + 1
    raycaster.setFromCamera(pointer, camera)
    const hit = raycaster.intersectObject(model, true)[0]
    return hit === undefined ? null : [hit.point.x, hit.point.y, hit.point.z]
  }

  /** The slot id of the marker under the pointer, or null. */
  function markerUnderPointer(event: PointerEvent): string | null {
    const rect = renderer.domElement.getBoundingClientRect()
    pointer.x = ((event.clientX - rect.left) / rect.width) * 2 - 1
    pointer.y = -((event.clientY - rect.top) / rect.height) * 2 + 1
    raycaster.setFromCamera(pointer, camera)
    const hit = raycaster.intersectObjects(markerSprites, false)[0]
    return hit === undefined ? null : ((hit.object.userData['slot'] as string | undefined) ?? null)
  }

  /** Ensures the loupe's border/crosshair overlay exists and is attached to the
   *  canvas's current parent — `render()` rebuilds that container each frame, so
   *  the div must be re-parented, not just created once. */
  function ensureLoupeEl(): void {
    const parent = renderer.domElement.parentElement
    if (parent === null) return
    if (loupeEl === null) {
      loupeEl = document.createElement('div')
      loupeEl.className = 'marker-loupe'
      loupeEl.hidden = true
    }
    if (loupeEl.parentElement !== parent) parent.appendChild(loupeEl)
  }

  /** Points the loupe at a surface point (or hides it when off the mesh). */
  function setLoupeTarget(point: [number, number, number] | null): void {
    if (!loupeActive) return
    if (point === null) {
      loupeShown = false
    } else {
      loupeTarget.set(point[0], point[1], point[2])
      loupeShown = true
    }
    if (loupeEl !== null) loupeEl.hidden = !loupeShown
  }

  /** Renders the magnified close-up into the top-right inset (scissored so it
   *  overwrites just that corner of the frame the main pass already drew). */
  function renderLoupe(): void {
    const size = renderer.getSize(loupeSize)
    // Top-right, below the nav toolbar. (WebGL's viewport origin is bottom-left,
    // so a "top" offset of LOUPE_TOP becomes size.y - LOUPE_SIZE - LOUPE_TOP.)
    const x = size.x - LOUPE_SIZE - LOUPE_MARGIN
    const y = size.y - LOUPE_SIZE - LOUPE_TOP
    loupeDir.copy(camera.position).sub(loupeTarget)
    const dist = loupeDir.length() || 1
    loupeCamera.position
      .copy(loupeTarget)
      .addScaledVector(loupeDir.multiplyScalar(1 / dist), dist / LOUPE_ZOOM)
    loupeCamera.up.copy(camera.up)
    loupeCamera.lookAt(loupeTarget)
    loupeCamera.updateProjectionMatrix()
    renderer.setScissorTest(true)
    renderer.setScissor(x, y, LOUPE_SIZE, LOUPE_SIZE)
    renderer.setViewport(x, y, LOUPE_SIZE, LOUPE_SIZE)
    renderer.render(scene, loupeCamera)
    renderer.setScissorTest(false)
    renderer.setViewport(0, 0, size.x, size.y)
  }

  function onMarkerPointerDown(event: PointerEvent): void {
    if (markerHandlers === null || event.button !== 0) return
    // Grab an existing marker to move it; otherwise drop the active one.
    const grabbed = markerUnderPointer(event)
    if (grabbed !== null) {
      draggingMarker = grabbed
      controls.enabled = false
      const point = markerSurfacePoint(event)
      if (point !== null) markerHandlers.onMove(grabbed, point)
      return
    }
    const point = markerSurfacePoint(event)
    if (point !== null) markerHandlers.onPlace(point)
  }

  function onMarkerPointerMove(event: PointerEvent): void {
    if (markerHandlers === null) return
    const point = markerSurfacePoint(event)
    setLoupeTarget(point)
    if (draggingMarker !== null) {
      if (point !== null) markerHandlers.onMove(draggingMarker, point)
      return
    }
    // A grab cursor over a marker, so it reads as draggable.
    renderer.domElement.style.cursor = markerUnderPointer(event) !== null ? 'grab' : ''
  }

  /** Hide the loupe when the pointer leaves the model area entirely. */
  function onMarkerPointerLeave(): void {
    setLoupeTarget(null)
  }

  function onMarkerPointerUp(): void {
    if (draggingMarker === null) return
    draggingMarker = null
    controls.enabled = true
  }

  return {
    canvas: renderer.domElement,

    async show(data: ArrayBuffer): Promise<ModelContents> {
      const contents = await parseModel(data)

      clearFittedEditing()
      setTransformMode('none')
      if (model !== null) {
        scene.remove(model)
        release(model)
        // importedSkeleton was a child of model, so release() disposed it.
        importedSkeleton = null
      }

      model = contents.root
      scene.add(model)
      if (contents.bones > 0) {
        importedSkeleton = importedBoneMesh(model)
        if (importedSkeleton !== null) model.add(importedSkeleton)
      }

      // Size the drawing buffer to the canvas now, rather than waiting for the
      // ResizeObserver — its timing differs across webviews, and a frame drawn
      // before it fires would draw into a default-sized (or zero) buffer.
      resize()

      bounds = contents.bounds
      // The grid's cell size follows the subject: metre cells under a 30 m
      // creature read as a postage stamp, and under a 20 cm one as a runway.
      const span = Math.max(bounds.getSize(new Vector3()).length(), 0.1)
      grid.setScale(span)

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
      // at a time so they do not z-fight. Carry any model reorientation across
      // so the preview stands the same way the posed mesh does.
      if (model !== null) {
        animated.position.copy(model.position)
        animated.quaternion.copy(model.quaternion)
        animated.scale.copy(model.scale)
        model.visible = false
      }
      // Hide the fitted skeleton while a clip plays: its octahedral bones sit at
      // the rest pose and would otherwise hang in the air over the moving mesh.
      if (fittedSkeleton !== null) fittedSkeleton.visible = false
      scene.add(animated)

      mixer = new AnimationMixer(animated)
      action = mixer.clipAction(clip)
      action.play()
      paused = false
      playing = true
      lastFrame = performance.now()
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
          // `toneMapped: false` so the baked debug hues (a bone's colour, the
          // red fallback flag) render exactly as computed — the scene's neutral
          // tone map would otherwise desaturate them and blur the flag.
          object.material = new MeshBasicMaterial({ vertexColors: true, toneMapped: false })
        }
      })
      if (model !== null) {
        overlay.position.copy(model.position)
        overlay.quaternion.copy(model.quaternion)
        overlay.scale.copy(model.scale)
        model.visible = false
      }
      // Hide the octahedral bones so they do not sit over the painted weights.
      if (fittedSkeleton !== null) fittedSkeleton.visible = false
      scene.add(overlay)
    },

    setPaused(value: boolean): void {
      paused = value
      // Re-anchor the delta clock so resuming does not jump by the paused span.
      lastFrame = performance.now()
    },

    setPlaybackDirection(direction): void {
      if (action !== null) action.timeScale = direction
    },

    seek(seconds): void {
      if (action === null || mixer === null) return
      action.time = Math.max(0, seconds)
      // Apply the new time to the pose without advancing (delta 0).
      mixer.update(0)
    },

    playbackTime(): number {
      return action?.time ?? 0
    },

    stop(): void {
      if (overlay !== null) {
        scene.remove(overlay)
        release(overlay)
        overlay = null
        if (model !== null) model.visible = true
      }
      playing = false
      paused = false
      action = null
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
      if (fittedSkeleton !== null) fittedSkeleton.visible = skeletonVisible
    },

    setSkeletonVisible(visible): void {
      skeletonVisible = visible
      // Only touch it when it's the thing on screen — a clip or the paint overlay
      // hides the bones regardless, and stop() re-reads the flag.
      if (fittedSkeleton !== null && !playing && overlay === null) {
        fittedSkeleton.visible = visible
      }
    },

    showFittedSkeleton(positions, parents, onEdit): void {
      clearFittedEditing()
      // The fitted rig supersedes the model's own; don't draw both.
      if (importedSkeleton !== null) importedSkeleton.visible = false
      if (fittedSkeleton !== null) {
        scene.remove(fittedSkeleton)
        fittedSkeleton.geometry.dispose()
        fittedSkeleton = null
      }

      const geometry = new BufferGeometry()
      fillBoneGeometry(geometry, positions, parents)
      if (geometry.getIndex()?.count === 0) {
        geometry.dispose()
        return
      }
      // Solid octahedral bones (see boneMaterial), drawn over the mesh rather
      // than through it: a skeleton the body hides is one nobody can check.
      fittedSkeleton = new Mesh(geometry, boneMaterial)
      fittedSkeleton.renderOrder = 1
      scene.add(fittedSkeleton)
      if (onEdit !== undefined) startFittedEditing(positions, parents, onEdit)
    },

    reframe,
    setView,
    zoom,
    setTransformMode,

    clearFittedSkeleton(): void {
      clearFittedEditing()
      if (fittedSkeleton !== null) {
        scene.remove(fittedSkeleton)
        fittedSkeleton.geometry.dispose()
        fittedSkeleton = null
      }
    },

    beginMarkerPlacement(handlers: MarkerHandlers): void {
      markerHandlers = handlers
      // The model's own rig would clutter placement; the new rig is what matters.
      if (importedSkeleton !== null) importedSkeleton.visible = false
      const dom = renderer.domElement
      // Idempotent: the app re-arms on every render, so drop any prior listeners.
      dom.removeEventListener('pointerdown', onMarkerPointerDown)
      dom.removeEventListener('pointermove', onMarkerPointerMove)
      dom.removeEventListener('pointerleave', onMarkerPointerLeave)
      window.removeEventListener('pointerup', onMarkerPointerUp)
      dom.addEventListener('pointerdown', onMarkerPointerDown)
      dom.addEventListener('pointermove', onMarkerPointerMove)
      dom.addEventListener('pointerleave', onMarkerPointerLeave)
      // On window, so releasing off the canvas still ends a drag.
      window.addEventListener('pointerup', onMarkerPointerUp)
      loupeActive = true
      ensureLoupeEl()
    },

    setMarkers(markers: readonly { id: string; position: readonly [number, number, number]; color: number }[]): void {
      for (const sprite of markerSprites) {
        scene.remove(sprite)
        sprite.material.dispose()
      }
      markerSprites = []
      // ~5% of the model's diagonal, so a ring reads clearly at any creature scale.
      const radius = Math.max(bounds.getSize(new Vector3()).length() * 0.038, 0.02)
      for (const marker of markers) {
        const material = new SpriteMaterial({
          map: ringTexture(marker.color),
          depthTest: false,
          transparent: true
        })
        const sprite = new Sprite(material)
        sprite.position.set(marker.position[0], marker.position[1], marker.position[2])
        sprite.scale.setScalar(radius)
        sprite.renderOrder = 3
        sprite.userData['slot'] = marker.id
        scene.add(sprite)
        markerSprites.push(sprite)
      }
    },

    endMarkerPlacement(): void {
      markerHandlers = null
      draggingMarker = null
      const dom = renderer.domElement
      dom.removeEventListener('pointerdown', onMarkerPointerDown)
      dom.removeEventListener('pointermove', onMarkerPointerMove)
      dom.removeEventListener('pointerleave', onMarkerPointerLeave)
      window.removeEventListener('pointerup', onMarkerPointerUp)
      dom.style.cursor = ''
      controls.enabled = true
      loupeActive = false
      loupeShown = false
      if (loupeEl !== null) loupeEl.hidden = true
      for (const sprite of markerSprites) {
        scene.remove(sprite)
        sprite.material.dispose()
      }
      markerSprites = []
      for (const texture of ringTextures.values()) texture.dispose()
      ringTextures.clear()
    },

    symmetryX(): number {
      return (bounds.min.x + bounds.max.x) * 0.5
    },

    info(): string {
      const el = renderer.domElement
      const size = renderer.getSize(new Vector2())
      return `canvas ${el.clientWidth}x${el.clientHeight} · buffer ${size.x}x${size.y} · frames ${renders} · webgl`
    },

    dispose(): void {
      playing = false
      setTransformMode('none')
      clearFittedEditing()
      if (overlay !== null) release(overlay)
      if (animated !== null) release(animated)
      if (model !== null) release(model)
      fittedSkeleton?.geometry.dispose()
      boneMaterial.dispose()
      grid.dispose()
      handleRest.dispose()
      handleHover.dispose()
      handleSelected.dispose()
      window.removeEventListener('keydown', onNumpadView)
      window.removeEventListener('wheel', onWheel, { capture: true })
      controls.dispose()
      renderer.dispose()
    }
  }
}
