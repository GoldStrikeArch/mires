// SHA3-256 and Keccak-256, adapted from @noble/hashes 2.4.0 (MIT).
let pi = [
  20, 14, 22, 34, 36, 6, 10, 32,
  16, 42, 48, 8, 30, 46, 38, 26,
  24, 4, 40, 28, 44, 18, 12, 2
]
let rotations = [
  1, 3, 6, 10, 15, 21, 28, 36,
  45, 55, 2, 14, 27, 41, 56, 8,
  25, 43, 62, 18, 39, 61, 20, 44
]
let rcLo = [
  1, 32898, 32906, -2147450880, 32907, -2147483647, -2147450751, 32777,
  138, 136, -2147450871, -2147483638, -2147450741, 139, 32905, 32771,
  32770, 128, 32778, -2147483638, -2147450751, 32896, -2147483647, -2147450872
]
let rcHi = [
  0, 0, -2147483648, -2147483648, 0, 0, -2147483648, -2147483648,
  0, 0, 0, 0, 0, -2147483648, -2147483648, -2147483648,
  -2147483648, -2147483648, 0, -2147483648, -2147483648, -2147483648, 0, -2147483648
]

external xor: (int, int) => int = "%xorint"
external band: (int, int) => int = "%andint"
external bor: (int, int) => int = "%orint"
external shl: (int, int) => int = "%lslint"
external shr: (int, int) => int = "%lsrint"

let permute = s => {
  let c = Array.make(~length=10, 0)
  for round in 0 to 23 {
    // Theta mixes column parities into every lane.
    for x in 0 to 9 {
      c[x] = xor(xor(xor(xor(Array.getUnsafe(s, x), Array.getUnsafe(s, x+10)), Array.getUnsafe(s, x+20)), Array.getUnsafe(s, x+30)), Array.getUnsafe(s, x+40))
    }
    for col in 0 to 4 {
      let x = 2 * col
      let left = Int.mod(x+8, 10)
      let right = Int.mod(x+2, 10)
      let a = Array.getUnsafe(c, right)
      let b = Array.getUnsafe(c, right+1)
      let lo = xor(Array.getUnsafe(c, left), bor(shl(a, 1), shr(b, 31)))
      let hi = xor(Array.getUnsafe(c, left+1), bor(shl(b, 1), shr(a, 31)))
      for row in 0 to 4 {
        let i = row * 10 + x
        s[i] = xor(Array.getUnsafe(s, i), lo)
        s[i+1] = xor(Array.getUnsafe(s, i+1), hi)
      }
    }
    // Rotate and move the 24 non-origin lanes in one cycle.
    let lo = ref(Array.getUnsafe(s, 2))
    let hi = ref(Array.getUnsafe(s, 3))
    for t in 0 to 23 {
      let shift = Array.getUnsafe(rotations, t)
      let a = if shift > 32 { hi.contents } else { lo.contents }
      let b = if shift > 32 { lo.contents } else { hi.contents }
      let k = band(shift, 31)
      let p = Array.getUnsafe(pi, t)
      lo := Array.getUnsafe(s, p)
      hi := Array.getUnsafe(s, p+1)
      s[p] = bor(shl(a, k), shr(b, 32-k))
      s[p+1] = bor(shl(b, k), shr(a, 32-k))
    }
    // Chi reads the original five lanes in each row.
    for row in 0 to 4 {
      let y = row * 10
      let b0 = Array.getUnsafe(s, y)
      let b1 = Array.getUnsafe(s, y+1)
      let b2 = Array.getUnsafe(s, y+2)
      let b3 = Array.getUnsafe(s, y+3)
      let chi = (a,b,c) => xor(a, band(xor(b,-1),c))
      for x in 0 to 5 {
        s[y+x] = chi(Array.getUnsafe(s, y+x), Array.getUnsafe(s, y+x+2), Array.getUnsafe(s, y+x+4))
      }
      s[y+6] = chi(Array.getUnsafe(s, y+6), Array.getUnsafe(s, y+8), b0)
      s[y+7] = chi(Array.getUnsafe(s, y+7), Array.getUnsafe(s, y+9), b1)
      s[y+8] = chi(Array.getUnsafe(s, y+8), b0, b2)
      s[y+9] = chi(Array.getUnsafe(s, y+9), b1, b3)
    }
    s[0] = xor(Array.getUnsafe(s, 0), Array.getUnsafe(rcLo, round))
    s[1] = xor(Array.getUnsafe(s, 1), Array.getUnsafe(rcHi, round))
  }
}

let digest = (legacy, chunks) => {
  let s = Array.make(~length=50, 0)
  let pos = ref(0)
  let xorByte = (i, byte) => {
    let word = shr(i, 2)
    s[word] = xor(Array.getUnsafe(s, word), shl(byte, 8 * band(i,3)))
  }
  chunks->Array.forEach(chunk => {
    let i = ref(0)
    while i.contents < Array.length(chunk) {
      if pos.contents == 0 && Array.length(chunk) - i.contents >= 136 {
        for word in 0 to 33 {
          let j = i.contents + word*4
          let data = bor(bor(Array.getUnsafe(chunk, j),shl(Array.getUnsafe(chunk, j+1),8)),bor(shl(Array.getUnsafe(chunk, j+2),16),shl(Array.getUnsafe(chunk, j+3),24)))
          s[word] = xor(Array.getUnsafe(s, word), data)
        }
        i := i.contents + 136
        permute(s)
      } else {
        xorByte(pos.contents, Array.getUnsafe(chunk, i.contents))
        pos := pos.contents + 1
        i := i.contents + 1
        if pos.contents == 136 {
          permute(s)
          pos := 0
        }
      }
    }
  })
  xorByte(pos.contents, if legacy {1} else {6})
  xorByte(135,128)
  permute(s)
  Array.fromInitializer(~length=32, i => band(shr(Array.getUnsafe(s, shr(i,2)), 8 * band(i,3)),255))
}
