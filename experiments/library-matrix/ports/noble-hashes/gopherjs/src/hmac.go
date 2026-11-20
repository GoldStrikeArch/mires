package main

// hmac is noble's _HMAC (hmac.ts) over the hash that create makes.
type hmac struct {
	iHash, oHash hash
}

func newHMAC(create func() hash, key []byte) *hmac {
	h := &hmac{iHash: create()}
	pad := make([]byte, h.iHash.blockLen())
	// A key longer than a block is replaced by its digest.
	if len(key) > len(pad) {
		long := create()
		long.update(key)
		key = long.digest()
	}
	copy(pad, key)
	for i := range pad {
		pad[i] ^= 0x36
	}
	h.iHash.update(pad)
	h.oHash = create()
	// Undo the inner XOR and apply the outer one.
	for i := range pad {
		pad[i] ^= 0x36 ^ 0x5c
	}
	h.oHash.update(pad)
	return h
}

func (h *hmac) blockLen() int      { return h.iHash.blockLen() }
func (h *hmac) outputLen() int     { return h.iHash.outputLen() }
func (h *hmac) update(data []byte) { h.iHash.update(data) }

func (h *hmac) digest() []byte {
	h.oHash.update(h.iHash.digest())
	return h.oHash.digest()
}
