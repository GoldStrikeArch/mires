// Translate the data, scales and value probes together to preserve the pixel-column workload.
export function variant(input, k) {
  // Split the nonce so even its largest safe value preserves adjacent sample coordinates.
  const radix = 2 ** 26
  const dx = k % radix
  const dy = dx + Math.floor(k / radix)
  return {...input, frames: input.frames.map(frame => ({
    ...frame,
    xRange: frame.xRange.map(x => x + dx),
    yRange: frame.yRange.map(y => y - dy),
    data: frame.data.map((values, index) => index === 0
      ? values.map(x => x + dx)
      : values.map(y => y === null ? null : y - dy)),
    probes: frame.probes.map(([x, y, px, py]) => [x + dx, y - dy, px, py])
  }))}
}
