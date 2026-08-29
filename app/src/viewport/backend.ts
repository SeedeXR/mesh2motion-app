/**
 * Render backend detection.
 *
 * ADR A1 assumes the viewport falls back to WebGL2 where WebGPU is
 * unavailable. Which one a given machine actually gets is a fact worth
 * surfacing: it changes performance characteristics, and a performance bug
 * report is much less useful without it.
 */

export type Backend = 'webgpu' | 'webgl2' | 'webgl' | 'none'

/**
 * Detects the best available render backend.
 *
 * WebGPU is probed by actually requesting an adapter — `navigator.gpu`
 * existing is not the same as a usable adapter being available.
 */
export async function detectBackend(): Promise<Backend> {
  const gpu = (navigator as { gpu?: { requestAdapter(): Promise<unknown> } }).gpu
  if (gpu !== undefined) {
    try {
      if ((await gpu.requestAdapter()) !== null) return 'webgpu'
    } catch {
      // Adapter request can reject outright; fall through to WebGL.
    }
  }

  const canvas = document.createElement('canvas')
  if (canvas.getContext('webgl2') !== null) return 'webgl2'
  if (canvas.getContext('webgl') !== null) return 'webgl'
  return 'none'
}
