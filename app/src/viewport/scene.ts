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
  Box3,
  Color,
  DirectionalLight,
  GridHelper,
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
import { applyFraming, frameBounds, parseModel, type ModelContents } from './model'

const FOV_DEGREES = 45

/** A mounted viewport. */
export interface Viewport {
  /** The canvas to place in the DOM. */
  readonly canvas: HTMLCanvasElement
  /** Draws a `.glb` from the bulk channel, replacing whatever was shown. */
  show(data: ArrayBuffer): Promise<ModelContents>
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

  /** Frees the GPU buffers an object tree holds. */
  function release(root: Object3D): void {
    root.traverse((object) => {
      if (!(object instanceof Mesh)) return
      object.geometry.dispose()
      const material: Material | Material[] = object.material
      for (const m of Array.isArray(material) ? material : [material]) m.dispose()
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

    reframe,

    dispose(): void {
      if (model !== null) release(model)
      skeleton?.dispose()
      controls.dispose()
      renderer.dispose()
    }
  }
}
