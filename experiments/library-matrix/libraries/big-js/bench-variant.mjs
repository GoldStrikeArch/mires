const modulus = 10n ** 16n

export function variant(input, k) {
  // The middle 16 digits have room for every safe integer nonce. Add to the
  // original digits so each operand retains its own coefficient pattern.
  const nonce = BigInt(k)
  function vary(digits) {
    const shifted = (BigInt(digits.slice(-17, -1)) + nonce) % modulus
    return digits.slice(0, -17) + String(shifted).padStart(16, '0') + digits.at(-1)
  }
  return {...input, cases:input.cases.map(c => ({...c, x:vary(c.x), y:vary(c.y)}))}
}
