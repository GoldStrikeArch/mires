// Canonical Huffman tree construction (hTree), 15/7-bit Kraft debt clamping,
// RLE code-length encoding (lc), and bit-cost helper (clen) matching fflate 0.8.3.

import 'dart:typed_data';

import 'tables.dart';

Uint16List hMap(Uint8List cd, int mb, bool reverse) =>
    buildHMap(cd, mb, reverse, tables.rev);

final class HuffmanTree {
  HuffmanTree(this.t, this.l);
  final Uint8List t;
  final int l;
}

final class CodeLengths {
  CodeLengths(this.c, this.n);
  final Uint16List c;
  final int n;
}

/// Builds canonical code lengths from symbol frequencies `d`, clamping depth to `mb`.
/// Matches fflate 0.8.3's UZIP.js two-queue merge and tie-breaks bit-for-bit.
HuffmanTree hTree(Uint16List d, int mb) {
  final dLen = d.length;
  var s = 0;
  for (var i = 0; i < dLen; ++i) {
    if (d[i] != 0) ++s;
  }
  if (s == 0) return HuffmanTree(tables.emptyU8, 0);

  if (s == 1) {
    for (var i = 0; i < dLen; ++i) {
      if (d[i] != 0) {
        final v = Uint8List(i + 1);
        v[i] = 1;
        return HuffmanTree(v, 1);
      }
    }
  }

  // Flat node arrays:
  //   0 .. s-1: leaves (initially in ascending symbol order)
  //   s:        sentinel leaf with freq 25001
  //   s+1 .. 2s-1: s-1 internal nodes in topological creation order
  final totalNodes = s * 2;
  final nodeS = Uint16List(s);
  final nodeF = Int32List(totalNodes);
  final nodeL = Int32List(totalNodes);
  final nodeR = Int32List(totalNodes);

  var idx = 0;
  var maxSym = 0;
  for (var i = 0; i < dLen; ++i) {
    final f = d[i];
    if (f != 0) {
      nodeS[idx] = i;
      nodeF[idx] = f;
      maxSym = i;
      ++idx;
    }
  }

  // Stable insertion sort by ascending frequency (ties preserve ascending symbol order).
  for (var i = 1; i < s; ++i) {
    final sym = nodeS[i];
    final freq = nodeF[i];
    var j = i - 1;
    while (j >= 0 && nodeF[j] > freq) {
      nodeS[j + 1] = nodeS[j];
      nodeF[j + 1] = nodeF[j];
      --j;
    }
    nodeS[j + 1] = sym;
    nodeF[j + 1] = freq;
  }

  // Sentinel beyond last leaf.
  nodeF[s] = 25001;

  // Two-queue linear merge (strict `<` picks leaf on equal frequency, matching fflate).
  nodeF[s + 1] = nodeF[0] + nodeF[1];
  nodeL[s + 1] = 0;
  nodeR[s + 1] = 1;
  var i0 = s + 1;
  var i1 = s + 2;
  var i2 = 2;
  while (i1 != totalNodes) {
    final int l;
    if (nodeF[i0] < nodeF[i2]) {
      l = i0++;
    } else {
      l = i2++;
    }
    final int r;
    if (i0 != i1 && nodeF[i0] < nodeF[i2]) {
      r = i0++;
    } else {
      r = i2++;
    }
    nodeF[i1] = nodeF[l] + nodeF[r];
    nodeL[i1] = l;
    nodeR[i1] = r;
    ++i1;
  }

  // Compute depths in reverse topological order (root is totalNodes - 1).
  final depth = Uint16List(totalNodes);
  for (var k = totalNodes - 1; k > s; --k) {
    final d1 = depth[k] + 1;
    depth[nodeL[k]] = d1;
    depth[nodeR[k]] = d1;
  }

  final tr = Uint16List(maxSym + 1);
  var mbt = 0;
  for (var k = 0; k < s; ++k) {
    final dk = depth[k];
    tr[nodeS[k]] = dk;
    if (dk > mbt) mbt = dk;
  }

  if (mbt > mb) {
    // Rebuild symbols in ascending order, then stably sort by
    // descending depth, ascending frequency, ascending symbol.
    final t2 = Uint16List(s);
    var ti = 0;
    for (var i = 0; i < dLen; ++i) {
      if (d[i] != 0) t2[ti++] = i;
    }
    for (var i = 1; i < s; ++i) {
      final sym = t2[i];
      final rDepth = tr[sym];
      final rFreq = d[sym];
      var j = i - 1;
      while (j >= 0) {
        final left = t2[j];
        final lDepth = tr[left];
        if (lDepth < rDepth || (lDepth == rDepth && d[left] > rFreq)) {
          t2[j + 1] = left;
          --j;
        } else {
          break;
        }
      }
      t2[j + 1] = sym;
    }

    var i = 0;
    var dt = 0;
    final lft = mbt - mb;
    final cst = 1 << lft;
    for (; i < s; ++i) {
      final sym = t2[i];
      final cur = tr[sym];
      if (cur > mb) {
        dt += cst - (1 << (mbt - cur));
        tr[sym] = mb;
      } else {
        break;
      }
    }
    dt >>= lft;
    while (dt > 0) {
      final sym = t2[i];
      final cur = tr[sym];
      if (cur < mb) {
        dt -= 1 << (mb - cur - 1);
        tr[sym] = cur + 1;
      } else {
        ++i;
      }
    }
    for (; i >= 0 && dt != 0; --i) {
      final sym = t2[i];
      if (tr[sym] == mb) {
        tr[sym] = mb - 1;
        ++dt;
      }
    }
    mbt = mb;
  }

  final out = Uint8List(maxSym + 1);
  for (var i = 0; i <= maxSym; ++i) {
    out[i] = tr[i];
  }
  return HuffmanTree(out, mbt);
}

/// RLE code-length encoding (`lc`), preserving fflate's empty-tree quirk (`{ c: [0], n: 1 }`).
CodeLengths lc(Uint8List c) {
  final cLen = c.length;
  var s = cLen;
  while (s > 0 && c[--s] == 0) {}
  ++s;
  final cl = Uint16List(s);
  var cli = 0;
  var cln = cLen > 0 ? c[0] : 0;
  var cls = 1;
  for (var i = 1; i <= s; ++i) {
    final nextVal = i < cLen ? c[i] : 0;
    if (i != s && nextVal == cln) {
      ++cls;
    } else {
      if (cln == 0 && cls > 2) {
        for (; cls > 138; cls -= 138) {
          cl[cli++] = 32754;
        }
        if (cls > 2) {
          cl[cli++] = cls > 10
              ? (((cls - 11) << 5) | 28690)
              : (((cls - 3) << 5) | 12305);
          cls = 0;
        }
      } else if (cls > 3) {
        cl[cli++] = cln;
        --cls;
        for (; cls > 6; cls -= 6) {
          cl[cli++] = 8304;
        }
        if (cls > 2) {
          cl[cli++] = ((cls - 3) << 5) | 8208;
          cls = 0;
        }
      }
      while (cls-- > 0) {
        cl[cli++] = cln;
      }
      cls = 1;
      cln = nextVal;
    }
  }
  return CodeLengths(Uint16List.sublistView(cl, 0, cli), s);
}

/// Bit cost of encoding symbols with frequencies `cf` and code lengths `cl`.
int clen(Uint16List cf, Uint8List cl) {
  var l = 0;
  final len = cl.length;
  for (var i = 0; i < len; ++i) {
    l += cf[i] * cl[i];
  }
  return l;
}
