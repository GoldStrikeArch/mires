import 'dart:math' as math;

// d3-hierarchy 3.1.3 layout algorithms. Child and contour links are arena indices.
class Node {
  String id = "";
  List<int> kids = [];
  int parent = -1;
  int depth = 0;
  int height = 0;
  int number = 0;
  int ancestor = 0;
  int defaultAncestor = -1;
  int thread = -1;
  double value = 0,
      x = 0,
      y = 0,
      r = 0,
      x0 = 0,
      y0 = 0,
      x1 = 0,
      y1 = 0,
      z = 0,
      m = 0,
      c = 0,
      s = 0;
}

Node circle(double x, double y, double r) => Node()
  ..x = x
  ..y = y
  ..r = r;
List<int> ints(int n) => List.filled(n, 0);
List<double> floats(int n) => List.filled(n, 0);
double real(int x) => x.toDouble();
int integer(double x) => x.toInt();
double minf(double a, double b) => math.min(a, b);
double maxf(double a, double b) => math.max(a, b);
double floorf(double x) => x.floorToDouble();
double roundf(double x) =>
    x >= -0.5 && x < 0 ? -0.0 : (x + 0.5).floorToDouble();
double sqrtf(double x) => math.sqrt(x);
double absf(double x) => x.abs();
int invalidBasis() => throw StateError('invalid enclosing basis');

double separation(List<Node> nodes, int a, int b) {
  return ((nodes[a].parent == nodes[b].parent) ? 1.0 : 2.0);
}

int contour(List<Node> nodes, int v, bool left) {
  final kids = nodes[v].kids;
  if ((kids.length == 0)) {
    return nodes[v].thread;
  }
  return (left ? kids[0] : kids[(kids.length - 1)]);
}

int apportion(List<Node> nodes, int v, int previous, int fallback) {
  int ancestor = fallback;
  if ((previous < 0)) {
    return ancestor;
  }
  int vip = v;
  int vop = v;
  int vim = previous;
  int vom = nodes[nodes[v].parent].kids[0];
  double sip = nodes[vip].m;
  double sop = nodes[vop].m;
  double sim = nodes[vim].m;
  double som = nodes[vom].m;
  vim = contour(nodes, vim, false);
  vip = contour(nodes, vip, true);
  while (((vim >= 0) && (vip >= 0))) {
    vom = contour(nodes, vom, true);
    vop = contour(nodes, vop, false);
    nodes[vop].ancestor = v;
    final shift =
        ((((nodes[vim].z + sim) - nodes[vip].z) - sip) +
        separation(nodes, vim, vip));
    if ((shift > 0.0)) {
      final anc = nodes[vim].ancestor;
      final wm = ((nodes[anc].parent == nodes[v].parent) ? anc : ancestor);
      final change = (shift / real((nodes[v].number - nodes[wm].number)));
      nodes[v].c = (nodes[v].c - change);
      nodes[v].s = (nodes[v].s + shift);
      nodes[wm].c = (nodes[wm].c + change);
      nodes[v].z = (nodes[v].z + shift);
      nodes[v].m = (nodes[v].m + shift);
      sip = (sip + shift);
      sop = (sop + shift);
    }
    sim = (sim + nodes[vim].m);
    sip = (sip + nodes[vip].m);
    som = (som + nodes[vom].m);
    sop = (sop + nodes[vop].m);
    vim = contour(nodes, vim, false);
    vip = contour(nodes, vip, true);
  }
  if (((vim >= 0) && (contour(nodes, vop, false) < 0))) {
    nodes[vop].thread = vim;
    nodes[vop].m = (nodes[vop].m + (sim - sop));
  }
  if (((vip >= 0) && (contour(nodes, vom, true) < 0))) {
    nodes[vom].thread = vip;
    nodes[vom].m = (nodes[vom].m + (sip - som));
    ancestor = v;
  }
  return ancestor;
}

void tidy(
  List<Node> nodes,
  List<int> pre,
  List<int> post,
  double w,
  double h,
  bool fixed,
) {
  final root = pre[0];
  for (final v in pre) {
    nodes[v].ancestor = v;
    nodes[v].thread = (-1);
    nodes[v].defaultAncestor = (-1);
    final kids = nodes[v].kids;
    for (var i = 0; i < kids.length; i++) {
      nodes[kids[i]].number = i;
    }
  }
  for (final v in post) {
    final n = nodes[v];
    final p = n.parent;
    int prev = (-1);
    if ((n.number > 0)) {
      prev = nodes[p].kids[(n.number - 1)];
    }
    final kids = n.kids;
    if ((kids.length > 0)) {
      double shift = 0.0;
      double change = 0.0;
      for (var i = kids.length - 1; i >= 0; i--) {
        final child = nodes[kids[i]];
        child.z = (child.z + shift);
        child.m = (child.m + shift);
        change = (change + child.c);
        shift = (shift + (child.s + change));
      }
      final mid = ((nodes[kids[0]].z + nodes[kids[(kids.length - 1)]].z) / 2.0);
      if ((prev < 0)) {
        n.z = mid;
      } else {
        n.z = (nodes[prev].z + separation(nodes, v, prev));
        n.m = (n.z - mid);
      }
    } else {
      if ((prev >= 0)) {
        n.z = (nodes[prev].z + separation(nodes, v, prev));
      }
    }
    if ((p >= 0)) {
      int ancestor = nodes[p].defaultAncestor;
      if ((ancestor < 0)) {
        ancestor = nodes[p].kids[0];
      }
      nodes[p].defaultAncestor = apportion(nodes, v, prev, ancestor);
    }
  }
  final offset = (-nodes[root].z);
  int left = root;
  int right = root;
  int depth = 0;
  for (final v in pre) {
    final n = nodes[v];
    final modifier = ((v == root) ? offset : nodes[n.parent].m);
    n.x = (n.z + modifier);
    n.m = (n.m + modifier);
    if ((n.x < nodes[left].x)) {
      left = v;
    }
    if ((n.x > nodes[right].x)) {
      right = v;
    }
    if ((n.depth > depth)) {
      depth = n.depth;
    }
  }
  final margin = ((left == right)
      ? 1.0
      : (separation(nodes, left, right) / 2.0));
  final tx = (margin - nodes[left].x);
  final kx = (w / ((nodes[right].x + margin) + tx));
  final ky = (h / real(((depth == 0) ? 1 : depth)));
  for (final v in pre) {
    final n = nodes[v];
    if (fixed) {
      n.x = (n.x * w);
      n.y = (real(n.depth) * h);
    } else {
      n.x = ((n.x + tx) * kx);
      n.y = (real(n.depth) * ky);
    }
  }
}

void cluster(
  List<Node> nodes,
  List<int> pre,
  List<int> post,
  double w,
  double h,
  bool fixed,
) {
  int previous = (-1);
  double x = 0.0;
  for (final v in post) {
    final n = nodes[v];
    if ((n.kids.length == 0)) {
      if ((previous >= 0)) {
        x = (x + separation(nodes, v, previous));
      }
      n.x = x;
      previous = v;
    } else {
      double height = 0.0;
      double total = 0.0;
      for (final c in n.kids) {
        total = (total + nodes[c].x);
        height = maxf(height, nodes[c].y);
      }
      n.x = (total / real(n.kids.length));
      n.y = (1.0 + height);
    }
  }
  final left = post[0];
  final right = previous;
  final x0 = (nodes[left].x - (separation(nodes, left, right) / 2.0));
  final x1 = (nodes[right].x + (separation(nodes, right, left) / 2.0));
  final rx = nodes[pre[0]].x;
  final ry = nodes[pre[0]].y;
  for (final v in post) {
    final n = nodes[v];
    if (fixed) {
      n.x = ((n.x - rx) * w);
      n.y = ((ry - n.y) * h);
    } else {
      n.x = (((n.x - x0) / (x1 - x0)) * w);
      n.y = ((1.0 - ((ry != 0.0) ? (n.y / ry) : 1.0)) * h);
    }
  }
}

void tileSlice(
  List<Node> nodes,
  List<int> kids,
  int start,
  int stop,
  double total,
  double x0,
  double y0,
  double x1,
  double y1,
  bool dice,
) {
  double x = x0;
  double y = y0;
  final scale = ((total == 0.0)
      ? total
      : (dice ? ((x1 - x) / total) : ((y1 - y) / total)));
  for (var i = start; i < stop; i++) {
    final n = nodes[kids[i]];
    n.x0 = x;
    n.y0 = y;
    n.x1 = x1;
    n.y1 = y1;
    if (dice) {
      x = (x + (n.value * scale));
      n.x1 = x;
    } else {
      y = (y + (n.value * scale));
      n.y1 = y;
    }
  }
}

void binary(
  List<Node> nodes,
  List<int> kids,
  List<double> sums,
  int start,
  int stop,
  double total,
  double x0,
  double y0,
  double x1,
  double y1,
) {
  if ((start >= (stop - 1))) {
    final n = nodes[kids[start]];
    n.x0 = x0;
    n.y0 = y0;
    n.x1 = x1;
    n.y1 = y1;
  } else {
    final off = sums[start];
    final target = ((total / 2.0) + off);
    int k = (start + 1);
    int hi = (stop - 1);
    while ((k < hi)) {
      final mid = ((k + hi) ~/ 2);
      if ((sums[mid] < target)) {
        k = (mid + 1);
      } else {
        hi = mid;
      }
    }
    if ((((target - sums[(k - 1)]) < (sums[k] - target)) &&
        ((start + 1) < k))) {
      k = (k - 1);
    }
    final left = (sums[k] - off);
    final right = (total - left);
    if (((x1 - x0) > (y1 - y0))) {
      final x = ((total != 0.0) ? (((x0 * right) + (x1 * left)) / total) : x1);
      binary(nodes, kids, sums, start, k, left, x0, y0, x, y1);
      binary(nodes, kids, sums, k, stop, right, x, y0, x1, y1);
    } else {
      final y = ((total != 0.0) ? (((y0 * right) + (y1 * left)) / total) : y1);
      binary(nodes, kids, sums, start, k, left, x0, y0, x1, y);
      binary(nodes, kids, sums, k, stop, right, x0, y, x1, y1);
    }
  }
}

void squarify(
  List<Node> nodes,
  List<int> kids,
  double value,
  double left,
  double top,
  double x1,
  double y1,
) {
  double total = value;
  double x = left;
  double y = top;
  int i = 0;
  int j = 0;
  final ratio = ((1.0 + sqrtf(5.0)) / 2.0);
  while ((i < kids.length)) {
    final dx = (x1 - x);
    final dy = (y1 - y);
    double row = nodes[kids[j]].value;
    j = (j + 1);
    while (((row == 0.0) && (j < kids.length))) {
      row = nodes[kids[j]].value;
      j = (j + 1);
    }
    double low = row;
    double high = row;
    final alpha = (maxf((dy / dx), (dx / dy)) / (total * ratio));
    double beta = ((row * row) * alpha);
    double best = maxf((high / beta), (beta / low));
    bool finished = false;
    while (((j < kids.length) && (!finished))) {
      final v = nodes[kids[j]].value;
      row = (row + v);
      low = minf(low, v);
      high = maxf(high, v);
      beta = ((row * row) * alpha);
      final score = maxf((high / beta), (beta / low));
      if ((score > best)) {
        row = (row - v);
        finished = true;
      } else {
        best = score;
        j = (j + 1);
      }
    }
    if ((dx < dy)) {
      final bottom = ((total != 0.0) ? (y + ((dy * row) / total)) : y1);
      tileSlice(nodes, kids, i, j, row, x, y, x1, bottom, true);
      if ((total != 0.0)) {
        y = bottom;
      }
    } else {
      final right = ((total != 0.0) ? (x + ((dx * row) / total)) : x1);
      tileSlice(nodes, kids, i, j, row, x, y, right, y1, false);
      if ((total != 0.0)) {
        x = right;
      }
    }
    total = (total - row);
    i = j;
  }
}

void rectangles(
  List<Node> nodes,
  List<int> pre,
  double w,
  double h,
  double p,
  bool partition,
  String tile,
  bool rounded,
) {
  final root = nodes[pre[0]];
  final levels = real((root.height + 1));
  root.x1 = w;
  root.y1 = h;
  if (partition) {
    root.x0 = p;
    root.y0 = p;
    root.y1 = (h / levels);
  }
  for (final v in pre) {
    final n = nodes[v];
    final kids = n.kids;
    final inset = ((n.parent >= 0) ? (p / 2.0) : 0.0);
    if (partition) {
      if ((kids.length > 0)) {
        tileSlice(
          nodes,
          kids,
          0,
          kids.length,
          n.value,
          n.x0,
          ((h * real((n.depth + 1))) / levels),
          n.x1,
          ((h * real((n.depth + 2))) / levels),
          true,
        );
      }
      n.x1 = (n.x1 - p);
      n.y1 = (n.y1 - p);
    } else {
      n.x0 = (n.x0 + inset);
      n.y0 = (n.y0 + inset);
      n.x1 = (n.x1 - inset);
      n.y1 = (n.y1 - inset);
    }
    if ((n.x1 < n.x0)) {
      n.x0 = ((n.x0 + n.x1) / 2.0);
      n.x1 = n.x0;
    }
    if ((n.y1 < n.y0)) {
      n.y0 = ((n.y0 + n.y1) / 2.0);
      n.y1 = n.y0;
    }
    if (((!partition) && (kids.length > 0))) {
      final gap = (p - (p / 2.0));
      double x = (n.x0 + gap);
      double y = (n.y0 + gap);
      double xx = (n.x1 - gap);
      double yy = (n.y1 - gap);
      if ((xx < x)) {
        x = ((x + xx) / 2.0);
        xx = x;
      }
      if ((yy < y)) {
        y = ((y + yy) / 2.0);
        yy = y;
      }
      if ((tile == "binary")) {
        final sums = floats((kids.length + 1));
        for (var j = 0; j < kids.length; j++) {
          sums[(j + 1)] = (sums[j] + nodes[kids[j]].value);
        }
        binary(nodes, kids, sums, 0, kids.length, n.value, x, y, xx, yy);
      } else {
        if ((tile == "sliceDice")) {
          tileSlice(
            nodes,
            kids,
            0,
            kids.length,
            n.value,
            x,
            y,
            xx,
            yy,
            ((n.depth % 2) == 0),
          );
        } else {
          squarify(nodes, kids, n.value, x, y, xx, yy);
        }
      }
    }
  }
  if (rounded) {
    for (final v in pre) {
      final n = nodes[v];
      n.x0 = roundf(n.x0);
      n.x1 = roundf(n.x1);
      n.y0 = roundf(n.y0);
      n.y1 = roundf(n.y1);
    }
  }
}

bool weak(Node a, Node b) {
  final dr = ((a.r - b.r) + (maxf(maxf(a.r, b.r), 1.0) * 1e-09));
  final dx = (b.x - a.x);
  final dy = (b.y - a.y);
  return ((dr > 0.0) && ((dr * dr) > ((dx * dx) + (dy * dy))));
}

bool outside(Node a, Node b) {
  final dr = (a.r - b.r);
  final dx = (b.x - a.x);
  final dy = (b.y - a.y);
  return ((dr < 0.0) || ((dr * dr) < ((dx * dx) + (dy * dy))));
}

Node pair(Node a, Node b) {
  final dx = (b.x - a.x);
  final dy = (b.y - a.y);
  final dr = (b.r - a.r);
  final distance = sqrtf(((dx * dx) + (dy * dy)));
  return circle(
    (((a.x + b.x) + ((dx / distance) * dr)) / 2.0),
    (((a.y + b.y) + ((dy / distance) * dr)) / 2.0),
    (((distance + a.r) + b.r) / 2.0),
  );
}

Node triple(Node a, Node b, Node c) {
  final a2 = (a.x - b.x);
  final a3 = (a.x - c.x);
  final b2 = (a.y - b.y);
  final b3 = (a.y - c.y);
  final c2 = (b.r - a.r);
  final c3 = (c.r - a.r);
  final d1 = (((a.x * a.x) + (a.y * a.y)) - (a.r * a.r));
  final d2 = (((d1 - (b.x * b.x)) - (b.y * b.y)) + (b.r * b.r));
  final d3 = (((d1 - (c.x * c.x)) - (c.y * c.y)) + (c.r * c.r));
  final ab = ((a3 * b2) - (a2 * b3));
  final xa = ((((b2 * d3) - (b3 * d2)) / (ab * 2.0)) - a.x);
  final xb = (((b3 * c2) - (b2 * c3)) / ab);
  final ya = ((((a3 * d2) - (a2 * d3)) / (ab * 2.0)) - a.y);
  final yb = (((a2 * c3) - (a3 * c2)) / ab);
  final qa = (((xb * xb) + (yb * yb)) - 1.0);
  final qb = (2.0 * ((a.r + (xa * xb)) + (ya * yb)));
  final qc = (((xa * xa) + (ya * ya)) - (a.r * a.r));
  final r = (-((absf(qa) > 1e-06)
      ? ((qb + sqrtf(((qb * qb) - ((4.0 * qa) * qc)))) / (2.0 * qa))
      : (qc / qb)));
  return circle(((a.x + xa) + (xb * r)), ((a.y + ya) + (yb * r)), r);
}

bool covers(List<Node> nodes, List<int> basis, int count, Node a) {
  int i = 0;
  while (((i < count) && weak(a, nodes[basis[i]]))) {
    i = (i + 1);
  }
  return (i == count);
}

int extend(List<Node> nodes, List<int> basis, int count, int p) {
  if (covers(nodes, basis, count, nodes[p])) {
    basis[0] = p;
    return 1;
  }
  for (var i = 0; i < count; i++) {
    final x = basis[i];
    if ((outside(nodes[p], nodes[x]) &&
        covers(nodes, basis, count, pair(nodes[x], nodes[p])))) {
      basis[0] = x;
      basis[1] = p;
      return 2;
    }
  }
  for (var i = 0; i < count; i++) {
    final x = basis[i];
    for (var j = (i + 1); j < count; j++) {
      final y = basis[j];
      if ((outside(pair(nodes[x], nodes[y]), nodes[p]) &&
          outside(pair(nodes[x], nodes[p]), nodes[y]) &&
          outside(pair(nodes[y], nodes[p]), nodes[x]) &&
          covers(nodes, basis, count, triple(nodes[x], nodes[y], nodes[p])))) {
        basis[0] = x;
        basis[1] = y;
        basis[2] = p;
        return 3;
      }
    }
  }
  return invalidBasis();
}

Node enclose(List<Node> nodes, List<int> front, int n, List<double> seed) {
  for (var j = n - 1; j >= 0; j--) {
    final step = ((1664525.0 * seed[0]) + 1013904223.0);
    seed[0] = (step - (floorf((step / 4294967296.0)) * 4294967296.0));
    final k = integer(((seed[0] / 4294967296.0) * real((j + 1))));
    final save = front[j];
    front[j] = front[k];
    front[k] = save;
  }
  final basis = ints(3);
  int count = 0;
  int i = 0;
  Node result = circle(0.0, 0.0, 0.0);
  while ((i < n)) {
    final p = front[i];
    if (((count > 0) && weak(result, nodes[p]))) {
      i = (i + 1);
    } else {
      count = extend(nodes, basis, count, p);
      if ((count == 1)) {
        result = circle(nodes[p].x, nodes[p].y, nodes[p].r);
      } else {
        if ((count == 2)) {
          result = pair(nodes[basis[0]], nodes[basis[1]]);
        } else {
          result = triple(nodes[basis[0]], nodes[basis[1]], nodes[basis[2]]);
        }
      }
      i = 0;
    }
  }
  return result;
}

void place(Node b, Node a, Node c) {
  final dx = (b.x - a.x);
  final dy = (b.y - a.y);
  final d2 = ((dx * dx) + (dy * dy));
  if ((d2 != 0.0)) {
    final a2 = ((a.r + c.r) * (a.r + c.r));
    final b2 = ((b.r + c.r) * (b.r + c.r));
    if ((a2 > b2)) {
      final x = (((d2 + b2) - a2) / (2.0 * d2));
      final y = sqrtf(maxf(0.0, ((b2 / d2) - (x * x))));
      c.x = ((b.x - (x * dx)) - (y * dy));
      c.y = ((b.y - (x * dy)) + (y * dx));
    } else {
      final x = (((d2 + a2) - b2) / (2.0 * d2));
      final y = sqrtf(maxf(0.0, ((a2 / d2) - (x * x))));
      c.x = ((a.x + (x * dx)) - (y * dy));
      c.y = ((a.y + (x * dy)) + (y * dx));
    }
  } else {
    c.x = (a.x + c.r);
    c.y = a.y;
  }
}

bool intersects(Node a, Node b) {
  final dr = ((a.r + b.r) - 1e-06);
  final dx = (b.x - a.x);
  final dy = (b.y - a.y);
  return ((dr > 0.0) && ((dr * dr) > ((dx * dx) + (dy * dy))));
}

double score(Node a, Node b) {
  final ab = (a.r + b.r);
  final dx = (((a.x * b.r) + (b.x * a.r)) / ab);
  final dy = (((a.y * b.r) + (b.y * a.r)) / ab);
  return ((dx * dx) + (dy * dy));
}

double siblings(List<Node> nodes, List<int> kids, List<double> seed) {
  final n = kids.length;
  int a = 0;
  int b = 1;
  nodes[kids[0]].x = 0.0;
  nodes[kids[0]].y = 0.0;
  if ((n == 1)) {
    return nodes[kids[0]].r;
  }
  nodes[kids[0]].x = (-nodes[kids[1]].r);
  nodes[kids[1]].x = nodes[kids[0]].r;
  nodes[kids[1]].y = 0.0;
  if ((n == 2)) {
    return (nodes[kids[0]].r + nodes[kids[1]].r);
  }
  place(nodes[kids[1]], nodes[kids[0]], nodes[kids[2]]);
  final after = ints(n);
  final before = ints(n);
  after[0] = 1;
  before[2] = 1;
  after[1] = 2;
  before[0] = 2;
  after[2] = 0;
  before[1] = 0;
  int i = 3;
  while ((i < n)) {
    place(nodes[kids[a]], nodes[kids[b]], nodes[kids[i]]);
    int j = after[b];
    int k = before[a];
    double sj = nodes[kids[b]].r;
    double sk = nodes[kids[a]].r;
    bool retry = false;
    bool more = true;
    while ((more && (!retry))) {
      if ((sj <= sk)) {
        if (intersects(nodes[kids[j]], nodes[kids[i]])) {
          b = j;
          after[a] = b;
          before[b] = a;
          retry = true;
        } else {
          sj = (sj + nodes[kids[j]].r);
          j = after[j];
        }
      } else {
        if (intersects(nodes[kids[k]], nodes[kids[i]])) {
          a = k;
          after[a] = b;
          before[b] = a;
          retry = true;
        } else {
          sk = (sk + nodes[kids[k]].r);
          k = before[k];
        }
      }
      more = (j != after[k]);
    }
    if ((!retry)) {
      before[i] = a;
      after[i] = b;
      after[a] = i;
      before[b] = i;
      b = i;
      double best = score(nodes[kids[a]], nodes[kids[after[a]]]);
      j = after[i];
      while ((j != b)) {
        final candidateScore = score(nodes[kids[j]], nodes[kids[after[j]]]);
        if ((candidateScore < best)) {
          best = candidateScore;
          a = j;
        }
        j = after[j];
      }
      b = after[a];
      i = (i + 1);
    }
  }
  final front = ints(n);
  front[0] = kids[b];
  int count = 1;
  int cursor = after[b];
  while ((cursor != b)) {
    front[count] = kids[cursor];
    count = (count + 1);
    cursor = after[cursor];
  }
  final enclosure = enclose(nodes, front, count, seed);
  for (final id in kids) {
    nodes[id].x = (nodes[id].x - enclosure.x);
    nodes[id].y = (nodes[id].y - enclosure.y);
  }
  return enclosure.r;
}

void packChildren(
  List<Node> nodes,
  List<int> post,
  double padding,
  List<double> seed,
) {
  final pad = ((padding != padding) ? 0.0 : padding);
  for (final i in post) {
    final kids = nodes[i].kids;
    if ((kids.length > 0)) {
      if ((pad != 0.0)) {
        for (final k in kids) {
          nodes[k].r = (nodes[k].r + pad);
        }
      }
      final radius = siblings(nodes, kids, seed);
      if ((pad != 0.0)) {
        for (final k in kids) {
          nodes[k].r = (nodes[k].r - pad);
        }
      }
      nodes[i].r = (radius + pad);
    }
  }
}

void pack(
  List<Node> nodes,
  List<int> pre,
  List<int> post,
  double w,
  double h,
  double p,
) {
  final root = nodes[pre[0]];
  root.x = (w / 2.0);
  root.y = (h / 2.0);
  for (final i in pre) {
    final n = nodes[i];
    if ((n.kids.length == 0)) {
      final raw = sqrtf(n.value);
      n.r = ((raw == raw) ? maxf(0.0, raw) : 0.0);
    }
  }
  final seed = floats(1);
  seed[0] = 1.0;
  packChildren(nodes, post, 0.0, seed);
  packChildren(nodes, post, (p * (root.r / minf(w, h))), seed);
  final k = (minf(w, h) / (2.0 * root.r));
  for (final i in pre) {
    final n = nodes[i];
    n.r = (n.r * k);
    if ((n.parent >= 0)) {
      n.x = (nodes[n.parent].x + (k * n.x));
      n.y = (nodes[n.parent].y + (k * n.y));
    }
  }
}
