/// Configuration options for `marked@18.1.1` (contract subset).
final class MarkedOptions {
  const MarkedOptions({
    this.gfm = true,
    this.breaks = false,
    this.pedantic = false,
  });

  final bool gfm;
  final bool breaks;
  final bool pedantic;
}
