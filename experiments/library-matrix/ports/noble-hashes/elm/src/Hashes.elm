module Hashes exposing (Hash, create, digest, update)

{-| The hashes reference.mjs runs, behind one type, and noble's hmac.ts over them.
-}

import Bitwise
import Blake2 exposing (Blake2, Options)
import Blake2b
import Blake2s
import Md exposing (Md)
import Sha2
import Sha3


type Hash
    = Sha256 (Md Sha2.Sha256)
    | Sha512 (Md Sha2.Sha512)
    | Keccak Sha3.Keccak
    | Blake2b (Blake2 Blake2b.State)
    | Blake2s (Blake2 Blake2s.State)
    | Hmac { iHash : Hash, oHash : Hash }


{-| reference.mjs's `hashes` table plus HMAC-SHA-256: the constructor for `algorithm`, or the
error noble throws for an unknown name.
-}
create : String -> Result String (Options -> Result String Hash)
create algorithm =
    case algorithm of
        "sha256" ->
            Ok (\_ -> Ok (Sha256 Sha2.sha256))

        "sha512" ->
            Ok (\_ -> Ok (Sha512 Sha2.sha512))

        "sha3_256" ->
            Ok (\_ -> Ok (Keccak (Sha3.keccak 136 0x06 32)))

        "keccak_256" ->
            Ok (\_ -> Ok (Keccak (Sha3.keccak 136 0x01 32)))

        "blake2b" ->
            Ok (Blake2b.blake2b >> Result.map Blake2b)

        "blake2s" ->
            Ok (Blake2s.blake2s >> Result.map Blake2s)

        "hmac_sha256" ->
            Ok (.key >> Result.fromMaybe "\"key\" expected Uint8Array" >> Result.map (hmac (\() -> Sha256 Sha2.sha256)))

        _ ->
            Err "unknown hash"


blockLen : Hash -> Int
blockLen hash =
    case hash of
        Sha256 md ->
            md.blockLen

        Sha512 md ->
            md.blockLen

        Keccak k ->
            k.blockLen

        Blake2b b ->
            b.blockLen

        Blake2s b ->
            b.blockLen

        Hmac { iHash } ->
            blockLen iHash


update : List Int -> Hash -> Hash
update data hash =
    case hash of
        Sha256 md ->
            Sha256 (Md.update data md)

        Sha512 md ->
            Sha512 (Md.update data md)

        Keccak k ->
            Keccak (Sha3.update data k)

        Blake2b b ->
            Blake2b (Blake2.update data b)

        Blake2s b ->
            Blake2s (Blake2.update data b)

        Hmac h ->
            Hmac { h | iHash = update data h.iHash }


digest : Hash -> List Int
digest hash =
    case hash of
        Sha256 md ->
            Md.digest md

        Sha512 md ->
            Md.digest md

        Keccak k ->
            Sha3.digest k

        Blake2b b ->
            Blake2.digest b

        Blake2s b ->
            Blake2.digest b

        Hmac { iHash, oHash } ->
            digest (update (digest iHash) oHash)


{-| noble's \_HMAC over the hash that `new` makes.
-}
hmac : (() -> Hash) -> List Int -> Hash
hmac new key =
    let
        iHash =
            new ()

        size =
            blockLen iHash

        -- A key longer than a block is replaced by its digest.
        short =
            if List.length key > size then
                digest (update key (new ()))

            else
                key

        pad =
            short ++ List.repeat (size - List.length short) 0
    in
    Hmac
        { iHash = update (List.map (Bitwise.xor 0x36) pad) iHash
        , oHash = update (List.map (Bitwise.xor 0x5C) pad) (new ())
        }
