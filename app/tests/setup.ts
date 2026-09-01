/**
 * three's GLTFLoader reaches for `self` on its texture path. Nothing here
 * renders, so a bare alias is enough to let a `.glb` parse under Node.
 */
;(globalThis as Record<string, unknown>)['self'] = globalThis
