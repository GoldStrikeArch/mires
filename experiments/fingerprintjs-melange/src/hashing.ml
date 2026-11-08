(* MurmurHash3 x64 128. The oracle mutates 32-bit JS lanes; OCaml `int`
   is signed 32-bit in Melange and silently corrupts the high bit, so the
   inner loop is the TypeScript body kept as JavaScript values. *)

open Host

let x64hash128_js : string -> int -> string =
  [%mel.raw
    {|
    function x64hash128(input, seed) {
      function getUTF8Bytes(input) {
        var result = new Uint8Array(input.length);
        for (var i = 0; i < input.length; i++) {
          var charCode = input.charCodeAt(i);
          if (charCode > 127) {
            return new TextEncoder().encode(input);
          }
          result[i] = charCode;
        }
        return result;
      }
      function x64Add(m, n) {
        var m0 = m[0] >>> 16, m1 = m[0] & 0xffff, m2 = m[1] >>> 16, m3 = m[1] & 0xffff;
        var n0 = n[0] >>> 16, n1 = n[0] & 0xffff, n2 = n[1] >>> 16, n3 = n[1] & 0xffff;
        var o0 = 0, o1 = 0, o2 = 0, o3 = 0;
        o3 += m3 + n3; o2 += o3 >>> 16; o3 &= 0xffff;
        o2 += m2 + n2; o1 += o2 >>> 16; o2 &= 0xffff;
        o1 += m1 + n1; o0 += o1 >>> 16; o1 &= 0xffff;
        o0 += m0 + n0; o0 &= 0xffff;
        m[0] = (o0 << 16) | o1;
        m[1] = (o2 << 16) | o3;
      }
      function x64Multiply(m, n) {
        var m0 = m[0] >>> 16, m1 = m[0] & 0xffff, m2 = m[1] >>> 16, m3 = m[1] & 0xffff;
        var n0 = n[0] >>> 16, n1 = n[0] & 0xffff, n2 = n[1] >>> 16, n3 = n[1] & 0xffff;
        var o0 = 0, o1 = 0, o2 = 0, o3 = 0;
        o3 += m3 * n3; o2 += o3 >>> 16; o3 &= 0xffff;
        o2 += m2 * n3; o1 += o2 >>> 16; o2 &= 0xffff;
        o2 += m3 * n2; o1 += o2 >>> 16; o2 &= 0xffff;
        o1 += m1 * n3; o0 += o1 >>> 16; o1 &= 0xffff;
        o1 += m2 * n2; o0 += o1 >>> 16; o1 &= 0xffff;
        o1 += m3 * n1; o0 += o1 >>> 16; o1 &= 0xffff;
        o0 += m0 * n3 + m1 * n2 + m2 * n1 + m3 * n0; o0 &= 0xffff;
        m[0] = (o0 << 16) | o1;
        m[1] = (o2 << 16) | o3;
      }
      function x64Rotl(m, bits) {
        var m0 = m[0];
        bits %= 64;
        if (bits === 32) { m[0] = m[1]; m[1] = m0; }
        else if (bits < 32) {
          m[0] = (m0 << bits) | (m[1] >>> (32 - bits));
          m[1] = (m[1] << bits) | (m0 >>> (32 - bits));
        } else {
          bits -= 32;
          m[0] = (m[1] << bits) | (m0 >>> (32 - bits));
          m[1] = (m0 << bits) | (m[1] >>> (32 - bits));
        }
      }
      function x64LeftShift(m, bits) {
        bits %= 64;
        if (bits === 0) return;
        else if (bits < 32) { m[0] = m[1] >>> (32 - bits); m[1] = m[1] << bits; }
        else { m[0] = m[1] << (bits - 32); m[1] = 0; }
      }
      function x64Xor(m, n) { m[0] ^= n[0]; m[1] ^= n[1]; }
      var F1 = [0xff51afd7, 0xed558ccd];
      var F2 = [0xc4ceb9fe, 0x1a85ec53];
      function x64Fmix(h) {
        var shifted = [0, h[0] >>> 1];
        x64Xor(h, shifted);
        x64Multiply(h, F1);
        shifted[1] = h[0] >>> 1;
        x64Xor(h, shifted);
        x64Multiply(h, F2);
        shifted[1] = h[0] >>> 1;
        x64Xor(h, shifted);
      }
      var C1 = [0x87c37b91, 0x114253d5];
      var C2 = [0x4cf5ad43, 0x2745937f];
      var M = [0, 5];
      var N1 = [0, 0x52dce729];
      var N2 = [0, 0x38495ab5];
      var key = getUTF8Bytes(input);
      seed = seed || 0;
      var length = [0, key.length];
      var remainder = length[1] % 16;
      var bytes = length[1] - remainder;
      var h1 = [0, seed];
      var h2 = [0, seed];
      var k1 = [0, 0];
      var k2 = [0, 0];
      var i;
      for (i = 0; i < bytes; i = i + 16) {
        k1[0] = key[i + 4] | (key[i + 5] << 8) | (key[i + 6] << 16) | (key[i + 7] << 24);
        k1[1] = key[i] | (key[i + 1] << 8) | (key[i + 2] << 16) | (key[i + 3] << 24);
        k2[0] = key[i + 12] | (key[i + 13] << 8) | (key[i + 14] << 16) | (key[i + 15] << 24);
        k2[1] = key[i + 8] | (key[i + 9] << 8) | (key[i + 10] << 16) | (key[i + 11] << 24);
        x64Multiply(k1, C1); x64Rotl(k1, 31); x64Multiply(k1, C2); x64Xor(h1, k1);
        x64Rotl(h1, 27); x64Add(h1, h2); x64Multiply(h1, M); x64Add(h1, N1);
        x64Multiply(k2, C2); x64Rotl(k2, 33); x64Multiply(k2, C1); x64Xor(h2, k2);
        x64Rotl(h2, 31); x64Add(h2, h1); x64Multiply(h2, M); x64Add(h2, N2);
      }
      k1[0] = 0; k1[1] = 0; k2[0] = 0; k2[1] = 0;
      var val = [0, 0];
      switch (remainder) {
        case 15: val[1] = key[i + 14]; x64LeftShift(val, 48); x64Xor(k2, val);
        case 14: val[1] = key[i + 13]; x64LeftShift(val, 40); x64Xor(k2, val);
        case 13: val[1] = key[i + 12]; x64LeftShift(val, 32); x64Xor(k2, val);
        case 12: val[1] = key[i + 11]; x64LeftShift(val, 24); x64Xor(k2, val);
        case 11: val[1] = key[i + 10]; x64LeftShift(val, 16); x64Xor(k2, val);
        case 10: val[1] = key[i + 9]; x64LeftShift(val, 8); x64Xor(k2, val);
        case 9:
          val[1] = key[i + 8];
          x64Xor(k2, val); x64Multiply(k2, C2); x64Rotl(k2, 33); x64Multiply(k2, C1); x64Xor(h2, k2);
        case 8: val[1] = key[i + 7]; x64LeftShift(val, 56); x64Xor(k1, val);
        case 7: val[1] = key[i + 6]; x64LeftShift(val, 48); x64Xor(k1, val);
        case 6: val[1] = key[i + 5]; x64LeftShift(val, 40); x64Xor(k1, val);
        case 5: val[1] = key[i + 4]; x64LeftShift(val, 32); x64Xor(k1, val);
        case 4: val[1] = key[i + 3]; x64LeftShift(val, 24); x64Xor(k1, val);
        case 3: val[1] = key[i + 2]; x64LeftShift(val, 16); x64Xor(k1, val);
        case 2: val[1] = key[i + 1]; x64LeftShift(val, 8); x64Xor(k1, val);
        case 1:
          val[1] = key[i];
          x64Xor(k1, val); x64Multiply(k1, C1); x64Rotl(k1, 31); x64Multiply(k1, C2); x64Xor(h1, k1);
      }
      x64Xor(h1, length); x64Xor(h2, length);
      x64Add(h1, h2); x64Add(h2, h1);
      x64Fmix(h1); x64Fmix(h2);
      x64Add(h1, h2); x64Add(h2, h1);
      return (
        ("00000000" + (h1[0] >>> 0).toString(16)).slice(-8) +
        ("00000000" + (h1[1] >>> 0).toString(16)).slice(-8) +
        ("00000000" + (h2[0] >>> 0).toString(16)).slice(-8) +
        ("00000000" + (h2[1] >>> 0).toString(16)).slice(-8)
      );
    }
  |}]

let x64hash128_seed (input : string) (seed : int) : string =
  x64hash128_js input seed

let x64hash128 (input : string) : string = x64hash128_seed input 0

let murmur (input : js) (seed : js) : js =
  let s = if is_undefined seed then 0 else to_int seed in
  of_string (x64hash128_seed (to_string input) s)
