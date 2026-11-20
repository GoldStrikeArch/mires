// The boundary's input: the corpus object itself.
type input = {
  algorithm: string,
  bytes?: array<int>,
  chunks?: array<array<int>>,
  repeat?: int,
  dkLen?: float,
  key?: array<int>,
  salt?: array<int>,
  personalization?: array<int>,
}

// reference.mjs's run: the lowercase hexadecimal digest of the input, or a throw where noble throws.
let run: input => string
