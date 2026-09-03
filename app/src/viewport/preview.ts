/**
 * A small looping clip preview — the moving thumbnail the clip chooser shows.
 *
 * Kept apart from `scene.ts`: this is a throwaway mini-viewport (one light, one
 * character, one clip at a time) that plays a library clip on the library
 * character so the user can see a motion before committing to it, the way
 * Mixamo's animated previews do. The whole library is loaded once; switching
 * clips is client-side, so hovering down a long list stays instant.
 */

import {
  type AnimationAction,
  type AnimationClip,
  AmbientLight,
  AnimationMixer,
  Box3,
  Color,
  DirectionalLight,
  type Material,
  Mesh,
  type Object3D,
  PerspectiveCamera,
  Scene,
  WebGLRenderer
} from 'three'
import { applyFraming, frameBounds, parseAnimated } from './model'

const FOV = 35

/** A mounted clip preview. */
export interface ClipPreview {
  /** The canvas to place in the DOM. */
  readonly canvas: HTMLCanvasElement
  /** Loads the library character and its clips; call once. */
  load(data: ArrayBuffer): Promise<void>
  /** Plays a clip by name, replacing whatever was playing. Unknown names are ignored. */
  play(clipName: string): void
  /** Releases the GPU resources. */
  dispose(): void
}

/** Frees the GPU buffers an object tree holds. */
function release(root: Object3D): void {
  root.traverse((object) => {
    if (!(object instanceof Mesh)) return
    object.geometry.dispose()
    const material = object.material as Material | Material[]
    for (const one of Array.isArray(material) ? material : [material]) one.dispose()
  })
}

export function createClipPreview(): ClipPreview {
  const renderer = new WebGLRenderer({ antialias: true })
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2))

  const scene = new Scene()
  scene.background = new Color(0x1a1c22)
  const camera = new PerspectiveCamera(FOV, 1, 0.01, 100)
  scene.add(new AmbientLight(0xffffff, 1.4))
  const key = new DirectionalLight(0xffffff, 1.6)
  key.position.set(2, 4, 3)
  scene.add(key)

  let root: Object3D | null = null
  let mixer: AnimationMixer | null = null
  let action: AnimationAction | null = null
  let clips: readonly AnimationClip[] = []
  let bounds = new Box3()
  let last = 0

  function resize(): void {
    const { clientWidth: w, clientHeight: h } = renderer.domElement
    if (w === 0 || h === 0) return
    renderer.setSize(w, h, false)
    camera.aspect = w / h
    camera.updateProjectionMatrix()
    if (root !== null) applyFraming(camera, frameBounds(bounds, FOV, camera.aspect))
  }
  new ResizeObserver(resize).observe(renderer.domElement)

  renderer.setAnimationLoop((now: number) => {
    if (mixer !== null) mixer.update((now - last) / 1000)
    last = now
    renderer.render(scene, camera)
  })

  return {
    canvas: renderer.domElement,

    async load(data): Promise<void> {
      const contents = await parseAnimated(data)
      if (root !== null) {
        scene.remove(root)
        release(root)
      }
      root = contents.root
      scene.add(root)
      clips = contents.clips
      bounds = contents.bounds
      mixer = new AnimationMixer(root)
      action = null
      resize()
    },

    play(clipName): void {
      if (mixer === null) return
      const clip = clips.find((c) => c.name === clipName)
      if (clip === undefined) return
      action?.stop()
      action = mixer.clipAction(clip)
      action.reset().play()
    },

    dispose(): void {
      renderer.setAnimationLoop(null)
      if (root !== null) release(root)
      renderer.dispose()
    }
  }
}
