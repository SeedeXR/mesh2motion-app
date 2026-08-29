/**
 * Headless GLB loading for benchmarks.
 *
 * GLTFLoader can parse geometry and skeletons in Node, but material loading
 * reaches for canvas/ImageBitmap decoding that Node has no implementation of,
 * so any textured model throws. The solver only ever reads positions and the
 * bone hierarchy, so the textures are stripped from the JSON chunk before
 * parsing rather than stubbing out three.js internals — that keeps this
 * independent of GLTFLoader's private details.
 */

import { readFileSync } from 'node:fs'
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js'
import type { Object3D } from 'three'

const GLB_MAGIC = 0x46546c67 // 'glTF' little-endian
const CHUNK_JSON = 0x4e4f534a // 'JSON'
const HEADER_BYTES = 12
const CHUNK_HEADER_BYTES = 8

interface GltfJson {
  images?: unknown[]
  textures?: unknown[]
  samplers?: unknown[]
  materials?: Record<string, unknown>[]
}

/**
 * Deletes every key containing "texture" at any depth.
 *
 * Texture slots hide inside material extensions too —
 * `KHR_materials_specular.specularColorTexture`,
 * `KHR_materials_clearcoat.clearcoatNormalTexture`, `KHR_materials_sheen.*`.
 * Hand-listing the levels leaves those dangling once `json.textures` is gone,
 * and GLTFLoader then dereferences a missing array. Recursing is both shorter
 * and correct for extensions that do not exist yet.
 */
function stripTextureKeys(node: unknown): void {
  if (Array.isArray(node)) {
    for (const item of node) stripTextureKeys(item)
    return
  }
  if (node === null || typeof node !== 'object') return

  const obj = node as Record<string, unknown>
  for (const key of Object.keys(obj)) {
    if (key.toLowerCase().includes('texture')) {
      delete obj[key]
    } else {
      stripTextureKeys(obj[key])
    }
  }
}

/** Removes every texture reference from a glTF JSON chunk, in place. */
function stripTextures(json: GltfJson): void {
  delete json.images
  delete json.textures
  delete json.samplers
  stripTextureKeys(json.materials)
}

/** Rebuilds a GLB with its JSON chunk replaced, preserving 4-byte alignment. */
function rewriteJsonChunk(original: Buffer, json: GltfJson): ArrayBuffer {
  const encoded = Buffer.from(JSON.stringify(json), 'utf8')
  // glTF requires chunks padded to 4 bytes; JSON chunks pad with spaces.
  const padding = (4 - (encoded.byteLength % 4)) % 4
  const jsonChunk = Buffer.concat([encoded, Buffer.alloc(padding, 0x20)])

  const originalJsonLength = original.readUInt32LE(HEADER_BYTES)
  const rest = original.subarray(HEADER_BYTES + CHUNK_HEADER_BYTES + originalJsonLength)

  const header = Buffer.alloc(HEADER_BYTES + CHUNK_HEADER_BYTES)
  header.writeUInt32LE(GLB_MAGIC, 0)
  header.writeUInt32LE(2, 4)
  header.writeUInt32LE(header.byteLength + jsonChunk.byteLength + rest.byteLength, 8)
  header.writeUInt32LE(jsonChunk.byteLength, 12)
  header.writeUInt32LE(CHUNK_JSON, 16)

  const out = Buffer.concat([header, jsonChunk, rest])
  return out.buffer.slice(out.byteOffset, out.byteOffset + out.byteLength) as ArrayBuffer
}

/** Loads a GLB from disk with textures stripped, returning its scene root. */
export async function loadGlbHeadless(path: string): Promise<Object3D> {
  const raw = readFileSync(path)
  if (raw.byteLength < HEADER_BYTES + CHUNK_HEADER_BYTES) {
    throw new Error(`${path} is too short to be a GLB`)
  }
  if (raw.readUInt32LE(0) !== GLB_MAGIC) {
    throw new Error(`${path} is not a GLB (bad magic)`)
  }
  if (raw.readUInt32LE(HEADER_BYTES + 4) !== CHUNK_JSON) {
    throw new Error(`${path}: first GLB chunk is not JSON`)
  }

  const jsonLength = raw.readUInt32LE(HEADER_BYTES)
  const jsonStart = HEADER_BYTES + CHUNK_HEADER_BYTES
  if (jsonStart + jsonLength > raw.byteLength) {
    // Fail here with something readable rather than letting JSON.parse chew on
    // binary and surface a confusing accessor error much later.
    throw new Error(
      `${path}: truncated — JSON chunk claims ${jsonLength} bytes, ` +
      `only ${raw.byteLength - jsonStart} remain`
    )
  }
  const json = JSON.parse(
    raw.subarray(jsonStart, jsonStart + jsonLength).toString('utf8')
  ) as GltfJson

  stripTextures(json)

  return await new Promise((resolve, reject) => {
    new GLTFLoader().parse(rewriteJsonChunk(raw, json), '', (g) => { resolve(g.scene) }, reject)
  })
}
