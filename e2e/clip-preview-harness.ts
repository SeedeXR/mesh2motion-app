// Renders the real clip preview (preview.ts) with a library glb at /anim.glb,
// playing its first clip, so a Playwright test can confirm the moving thumbnail
// actually draws the library character.
import { createClipPreview } from '../app/src/viewport/preview'
import { parseAnimated } from '../app/src/viewport/model'

const win = window as unknown as Record<string, unknown>
const preview = createClipPreview()
document.getElementById('app')!.appendChild(preview.canvas)

async function run(): Promise<void> {
  const bytes = await (await fetch('/anim.glb')).arrayBuffer()
  await preview.load(bytes)
  const contents = await parseAnimated(bytes.slice(0))
  const name = contents.clips[0]?.name
  if (name !== undefined) preview.play(name)
  win.__ready = true
}

run().catch((e) => {
  win.__error = String(e)
  win.__ready = true
})
