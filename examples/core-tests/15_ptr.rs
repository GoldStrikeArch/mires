//! `core::ptr`, which is the pointer model's own unit test.
//!
//! Every form a pointer can take is here: a slot into an array, a slot naming a struct field, a
//! boxed local, a byte view produced by a cast, and the three provenance free numbers
//! (`null()`, a dangling alignment, `without_provenance`). What is checked is not the addresses —
//! a synthetic base is not a real one and nothing may depend on its value — but the *relations*
//! the model promises: a real pointer is never null, a dangling one never collides with a real
//! buffer, the difference of two element pointers is their index difference, a byte view of a
//! pointer has the same address as the pointer, and a copy of an aggregate is a value rather than
//! an alias.
//!
//! The few absolute numbers printed are the ones that are absolute in Rust too: the alignment
//! `dangling()` returns, and an address a program made up itself with `without_provenance`.

#![no_std]
#![no_main]

use core::mem::align_of;
use core::ptr;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy, PartialEq)]
struct Pair {
    first: i32,
    second: i32,
}

fn print_i32s(values: &[i32]) {
    let mut index = 0;
    while index < values.len() {
        print_i32(values[index]);
        index += 1;
    }
}

fn print_u8s(values: &[u8]) {
    let mut index = 0;
    while index < values.len() {
        print_i32(values[index] as i32);
        index += 1;
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    // ------------------------------------------------------------------------ null and is_null
    let nothing: *const i32 = ptr::null();
    let nothing_mut: *mut i32 = ptr::null_mut();
    print_bool(nothing.is_null());
    print_bool(nothing_mut.is_null());
    print_usize(nothing.addr());
    print_bool(ptr::eq(nothing, ptr::null()));

    let value = 7i32;
    let to_value: *const i32 = &value;
    print_bool(to_value.is_null());
    unsafe { print_i32(*to_value) }

    let values = [10i32, 20, 30, 40, 50];
    print_bool(values.as_ptr().is_null());
    print_bool(<[i32]>::as_ptr(&[]).is_null());

    // ------------------------------------------------------------------------- a dangling pointer
    // `NonNull::dangling()` itself is out — it is `transmute::<Alignment, NonNull<T>>`, and an
    // enum whose discriminant *is* its value has no transmute lowering yet — so the same pointer
    // is built the way `dangling` documents it: the type's alignment, as an address. What matters
    // is the relation the model promises, that such an address is below every synthetic buffer
    // base and so can never be mistaken for a real pointer.
    let dangling_u8: *const u8 = ptr::without_provenance(align_of::<u8>());
    let dangling_i32: *const i32 = ptr::without_provenance(align_of::<i32>());
    let dangling_u64: *const u64 = ptr::without_provenance(align_of::<u64>());
    print_usize(dangling_u8.addr());
    print_usize(dangling_i32.addr());
    print_usize(dangling_u64.addr());
    print_bool(dangling_i32.is_null());
    print_bool(dangling_i32.addr() < 4096);
    print_bool(dangling_i32.is_aligned());

    // ---------------------------------------------------- without_provenance and the addr round trip
    let synthetic: *const u8 = ptr::without_provenance(0x1234);
    print_usize(synthetic.addr());
    print_bool(synthetic.is_null());
    print_usize(ptr::without_provenance::<i32>(64).addr());
    print_usize(ptr::without_provenance::<u8>(0).addr());
    print_bool(ptr::without_provenance::<u8>(0).is_null());
    // A wrapping offset is the one a provenance free pointer may take, and it moves by whole
    // elements.
    print_usize(synthetic.wrapping_add(4).addr());
    print_usize(ptr::without_provenance::<i32>(64).wrapping_add(2).addr());
    print_usize(ptr::without_provenance::<i32>(64).wrapping_sub(1).addr());

    // ----------------------------------------------------------- ptr::eq, and identity of a raw
    // Two raw pointers taken from one object are one record, so they are equal; two taken from two
    // equal objects are not.
    let pair = Pair { first: 1, second: 2 };
    let same = Pair { first: 1, second: 2 };
    let to_pair: *const Pair = &pair;
    let to_pair_again: *const Pair = &pair;
    let to_same: *const Pair = &same;
    print_bool(ptr::eq(to_pair, to_pair_again));
    print_bool(ptr::eq(to_pair, to_same));
    print_bool(to_pair == to_pair_again);
    print_bool(to_pair != to_same);
    print_bool(pair == same);

    let boxed = 5i32;
    let to_boxed: *const i32 = &boxed;
    let to_boxed_again: *const i32 = &boxed;
    print_bool(ptr::eq(to_boxed, to_boxed_again));
    print_bool(ptr::eq(to_boxed, to_value));

    let base = values.as_ptr();
    print_bool(ptr::eq(base, values.as_ptr()));
    print_bool(ptr::eq(base, unsafe { base.add(1) }));
    // A fat pointer equals the thin pointer to its start: `ptr_eq` ignores the length.
    print_bool(ptr::eq((&values[..] as *const [i32]).cast::<i32>(), base));
    print_bool(ptr::eq(base, &values[0]));

    // ------------------------------------------------------- offset, add, sub, offset_from
    unsafe {
        print_i32(*base);
        print_i32(*base.add(2));
        print_i32(*base.offset(4));
        print_i32(*base.add(4).sub(1));
        print_i32(base.add(3).read());
        print_i32(base.add(2).offset_from(base) as i32);
        print_i32(base.offset_from(base.add(2)) as i32);
        print_i32(base.offset_from(base) as i32);
        print_i32(base.add(2).byte_offset_from(base) as i32);
    }

    // A pointer walked with `add` reads the same elements the index does.
    let mut walked = 0;
    let mut step = 0;
    while step < values.len() {
        walked += unsafe { *base.add(step) };
        step += 1;
    }
    print_i32(walked);

    // `from_raw_parts` rebuilds a slice out of a pointer and a length.
    let tail = unsafe { core::slice::from_raw_parts(base.add(1), 3) };
    print_usize(tail.len());
    print_i32(tail[0]);
    print_i32(tail[2]);
    print_i32(tail.iter().sum::<i32>());

    // ---------------------------------------------------------------------------- read and write
    let mut buffer = [1i32, 2, 3, 4];
    let writable = buffer.as_mut_ptr();
    unsafe {
        ptr::write(writable, 100);
        ptr::write(writable.add(3), 400);
        print_i32(ptr::read(writable));
        print_i32(ptr::read(writable.add(3)));
        *writable.add(1) = 200;
        print_i32(writable.add(1).read());
    }
    print_i32s(&buffer);

    // A write through a pointer is seen by every alias of the place.
    let mut aliased = 1i32;
    let first_alias: *mut i32 = &mut aliased;
    let second_alias: *mut i32 = first_alias;
    unsafe { ptr::write(second_alias, 9) };
    print_i32(aliased);
    print_i32(unsafe { ptr::read(first_alias) });

    // A struct written through a pointer.
    let mut record = Pair { first: 1, second: 2 };
    let to_record: *mut Pair = &mut record;
    unsafe { ptr::write(to_record, Pair { first: 3, second: 4 }) };
    print_i32(record.first + record.second);
    print_i32(unsafe { ptr::read(to_record) }.first);

    // ------------------------------------------------------------- copy and copy_nonoverlapping
    let source = [1i32, 2, 3, 4];
    let mut destination = [0i32; 4];
    unsafe { ptr::copy_nonoverlapping(source.as_ptr(), destination.as_mut_ptr(), 4) };
    print_i32s(&destination);
    unsafe { ptr::copy_nonoverlapping(source.as_ptr(), destination.as_mut_ptr(), 0) };
    print_i32s(&destination);

    // Overlapping, forwards: the destination is above the source, so a naive loop would smear.
    let mut forward = [1i32, 2, 3, 4, 5, 6];
    unsafe {
        let start = forward.as_mut_ptr();
        ptr::copy(start, start.add(2), 4);
    }
    print_i32s(&forward);

    // Overlapping, backwards.
    let mut backward = [1i32, 2, 3, 4, 5, 6];
    unsafe {
        let start = backward.as_mut_ptr();
        ptr::copy(start.add(2), start, 4);
    }
    print_i32s(&backward);

    // A copy onto itself, and a copy of nothing.
    let mut untouched = [1i32, 2, 3];
    unsafe {
        let start = untouched.as_mut_ptr();
        ptr::copy(start, start, 3);
        ptr::copy(start, start.add(1), 0);
    }
    print_i32s(&untouched);

    // Bytes, which is the pointee the byte oriented helpers are written for.
    let mut byte_buffer = [1u8, 2, 3, 4, 5, 6];
    unsafe {
        let start = byte_buffer.as_mut_ptr();
        ptr::copy(start, start.add(2), 3);
    }
    print_u8s(&byte_buffer);

    // An aggregate element crosses as a value, not as an alias: writing to the copy must not
    // change the original.
    let pairs = [Pair { first: 1, second: 2 }, Pair { first: 3, second: 4 }];
    let mut copied = [Pair { first: 0, second: 0 }; 2];
    unsafe { ptr::copy_nonoverlapping(pairs.as_ptr(), copied.as_mut_ptr(), 2) };
    print_i32(copied[0].first + copied[1].second);
    copied[0].first = 99;
    print_i32(pairs[0].first);
    print_i32(copied[0].first);

    // ----------------------------------------------------------------------------- write_bytes
    let mut pattern = [1u8, 2, 3, 4];
    unsafe { ptr::write_bytes(pattern.as_mut_ptr(), 7, 3) };
    print_u8s(&pattern);
    unsafe { ptr::write_bytes(pattern.as_mut_ptr().add(3), 0, 1) };
    print_u8s(&pattern);

    let mut zeroed = [5i32; 4];
    unsafe { ptr::write_bytes(zeroed.as_mut_ptr(), 0, 2) };
    print_i32s(&zeroed);

    // --------------------------------------------------------------------------------- swap
    let mut left = 1i32;
    let mut right = 2i32;
    unsafe { ptr::swap(&mut left, &mut right) };
    print_i32(left);
    print_i32(right);

    let mut swapped = [1i32, 2, 3, 4];
    unsafe {
        let start = swapped.as_mut_ptr();
        ptr::swap(start, start.add(3));
    }
    print_i32s(&swapped);

    // An aggregate swapped through two raw pointers, which is a whole value write through each of
    // them and not a field write. `[T]::swap` is the same thing reached from safe code, so this
    // pair of assertions is the regression test for a write through a raw pointer to a bare
    // aggregate being seen by the place the pointer was taken from.
    let mut one = Pair { first: 1, second: 2 };
    let mut two = Pair { first: 3, second: 4 };
    unsafe { ptr::swap(&mut one, &mut two) };
    print_i32(one.first);
    print_i32(two.first);

    let mut records = [Pair { first: 1, second: 2 }, Pair { first: 3, second: 4 }];
    records.swap(0, 1);
    print_i32(records[0].first);
    print_i32(records[1].first);
    print_i32(records[0].second);

    // The same write, spelled as an assignment through the pointer.
    let mut whole = Pair { first: 1, second: 2 };
    let to_whole: *mut Pair = &mut whole;
    unsafe { *to_whole = Pair { first: 5, second: 6 } };
    print_i32(whole.first);
    print_i32(whole.second);

    // ------------------------------------------------------------- &raw of a struct field
    let mut fields = Pair { first: 1, second: 2 };
    let to_second = &raw mut fields.second;
    unsafe {
        *to_second = 20;
        print_i32(ptr::read(to_second));
        ptr::write(to_second, 30);
    }
    print_i32(fields.second);
    print_i32(fields.first);
    let to_first = &raw const fields.first;
    print_i32(unsafe { *to_first });
    print_bool(ptr::eq(to_first, &raw const fields.first));
    print_bool(ptr::eq(to_first.cast::<i32>(), to_second));

    // ---------------------------------------------------------- the *const T -> *const u8 chain
    let numbers = [1i32, 2, 3];
    let to_zeroth: *const i32 = &numbers[0];
    let to_first_number: *const i32 = &numbers[1];
    print_usize(to_first_number.addr() - to_zeroth.addr());
    print_bool(to_zeroth.is_aligned());
    print_usize(to_zeroth.addr() % 4);

    let as_bytes = to_zeroth as *const u8;
    let as_bytes_next = to_first_number as *const u8;
    print_bool(as_bytes.addr() == to_zeroth.addr());
    print_usize(as_bytes_next.addr() - as_bytes.addr());
    print_bool(as_bytes.is_null());
    print_usize(unsafe { as_bytes.add(4).addr() } - as_bytes.addr());
    print_bool(as_bytes.cast::<i32>().addr() == to_zeroth.addr());

    // A whole slice's byte view, which is what `as_bytes` on a `str` produces too.
    let text = "hi";
    let text_bytes = text.as_bytes();
    print_usize(text_bytes.len());
    print_i32(text_bytes[0] as i32);
    print_bool(text_bytes.as_ptr().is_null());
    print_usize(unsafe { text_bytes.as_ptr().add(1).read() } as usize);

    // -------------------------------------------------------------- array equality via raw_eq
    let bytes_a = [1u8, 2, 3, 4];
    let bytes_b = [1u8, 2, 3, 4];
    let bytes_c = [1u8, 2, 3, 5];
    print_bool(bytes_a == bytes_b);
    print_bool(bytes_a == bytes_c);
    print_bool(bytes_a != bytes_c);

    let ints_a = [1i32, 2, 3, 4];
    let ints_b = [1i32, 2, 3, 4];
    let ints_c = [1i32, 2, 3, 5];
    print_bool(ints_a == ints_b);
    print_bool(ints_a == ints_c);
    print_bool(ints_a != ints_c);

    // And the same comparison over slices, which is the bytewise path.
    print_bool(bytes_a[..] == bytes_b[..]);
    print_bool(bytes_a[..] == bytes_c[..]);
    print_bool(ints_a[..] == ints_b[..]);
    print_bool(ints_a[..] == ints_c[..]);
    let empty: &[u8] = &[];
    print_bool(empty == empty);

    // ------------------------------------------------------------ read_unaligned/write_unaligned
    // Both are written in `core` as a cast to `*const Unaligned<T>` — a `#[repr(packed)]` newtype
    // that exists to carry `T`'s bytes at alignment 1. A packed newtype holds exactly its field's
    // bytes, and this model has no alignment for it to differ in, so the wrapper *is* its field:
    // the cast is the identity, the read hands back the element, and the write stores it.
    //
    // What the two do *not* do is reinterpret a buffer. Reading four bytes of a `[u8; N]` as a
    // `u32` asks for an element that is not in that buffer, and `__rt.unscale` refuses it at the
    // cast whether the read is aligned or not; `u32::from_le_bytes` is the spelling that works,
    // and it is below.
    let mut words = [10u32, 20, 30, 40];
    let words_ptr = words.as_ptr();
    print_u32(unsafe { ptr::read_unaligned(words_ptr) });
    print_u32(unsafe { ptr::read_unaligned(words_ptr.add(2)) });
    print_u32(unsafe { words_ptr.add(3).read_unaligned() });

    let words_mut = words.as_mut_ptr();
    unsafe { ptr::write_unaligned(words_mut.add(1), 99) };
    unsafe { words_mut.add(3).write_unaligned(77) };
    print_u32(words[0]);
    print_u32(words[1]);
    print_u32(words[3]);
    // A round trip through both, at an offset, leaves the value and its neighbours alone.
    let taken = unsafe { ptr::read_unaligned(words_ptr.add(2)) };
    unsafe { ptr::write_unaligned(words_mut.add(2), taken + 5) };
    print_u32(words[1]);
    print_u32(words[2]);
    print_u32(words[3]);

    // The same pair over an aggregate pointee, where the wrapper's field is an object rather than
    // a number and the write has to reach the element rather than rebind a copy of it.
    let mut pairs = [Pair { first: 1, second: 2 }, Pair { first: 3, second: 4 }];
    let pairs_ptr = pairs.as_ptr();
    let read_pair = unsafe { ptr::read_unaligned(pairs_ptr.add(1)) };
    print_i32(read_pair.first);
    print_i32(read_pair.second);
    let pairs_mut = pairs.as_mut_ptr();
    unsafe { ptr::write_unaligned(pairs_mut, Pair { first: 7, second: 8 }) };
    print_i32(pairs[0].first);
    print_i32(pairs[0].second);
    print_i32(pairs[1].first);
    // The value read out is a copy: writing through the pointer afterwards does not change it.
    print_i32(read_pair.first + read_pair.second);

    // The byte punning `read_unaligned` is often reached for, spelled the way this model can
    // answer: the bytes are named individually and assembled by `from_le_bytes`.
    let raw_bytes = [1u8, 0, 0, 0, 2, 0, 0, 0, 255, 255, 0, 0];
    print_u32(u32::from_le_bytes([raw_bytes[0], raw_bytes[1], raw_bytes[2], raw_bytes[3]]));
    print_u32(u32::from_le_bytes([raw_bytes[4], raw_bytes[5], raw_bytes[6], raw_bytes[7]]));
    print_u32(u32::from_le_bytes([raw_bytes[8], raw_bytes[9], raw_bytes[10], raw_bytes[11]]));

    // The same conversion at eight bytes, where the integer is a `BigInt` rather than a number and
    // the int32 operators would throw the top half away. The high byte and the negative case are
    // what tell a widened conversion from a truncated one.
    print_u64(u64::from_le_bytes([1, 0, 0, 0, 0, 0, 0, 0]));
    print_u64(u64::from_le_bytes([0, 0, 0, 0, 0, 0, 0, 1]));
    print_u64(u64::from_le_bytes([255, 255, 255, 255, 255, 255, 255, 255]));
    print_i64(i64::from_le_bytes([255, 255, 255, 255, 255, 255, 255, 255]));
    print_i64(i64::from_le_bytes([254, 255, 255, 255, 255, 255, 255, 255]));
    let eight = 258u64.to_le_bytes();
    print_i32(eight[0] as i32);
    print_i32(eight[1] as i32);
    print_i32(eight[7] as i32);
    let high = (1u64 << 56).to_le_bytes();
    print_i32(high[7] as i32);
    print_i32(high[0] as i32);
    let negative = (-2i64).to_le_bytes();
    print_i32(negative[0] as i32);
    print_i32(negative[7] as i32);
    // Round trips, which is the property the two directions owe each other.
    print_u64(u64::from_le_bytes(0x0123_4567_89ab_cdefu64.to_le_bytes()));
    print_i64(i64::from_le_bytes((-1234567890123i64).to_le_bytes()));
    print_u64(u64::from_be_bytes(0x0123_4567_89ab_cdefu64.to_be_bytes()));

    // ------------------------------------------------------- byte_offset / byte_add / byte_sub
    // All three are `self.cast::<u8>().offset(n).with_metadata_of(self)`, and that last step is
    // `from_raw_parts` with `()` for its metadata — a thin pointer rebuilt from a data pointer
    // that arrived erased to `*const ()`. For a `*const u8` the erased value is already the slot;
    // for a `*const i32` it is the scaled byte view `cast::<u8>()` produced, and keying it back
    // into elements is the whole of the round trip.
    let byte_run = [10u8, 20, 30, 40, 50, 60];
    let byte_start = byte_run.as_ptr();
    print_i32(unsafe { *byte_start.byte_add(1) } as i32);
    print_i32(unsafe { *byte_start.byte_offset(3) } as i32);
    print_i32(unsafe { *byte_start.byte_add(5).byte_sub(2) } as i32);
    print_i32(unsafe { *byte_start.byte_offset(0) } as i32);
    print_bool(unsafe { byte_start.byte_add(2) } == unsafe { byte_start.add(2) });

    // The scaled case: a byte offset over four byte elements only names an element when it is a
    // multiple of four, and every one of these is.
    let int_run = [11i32, 22, 33, 44, 55];
    let int_start = int_run.as_ptr();
    print_i32(unsafe { *int_start.byte_add(4) });
    print_i32(unsafe { *int_start.byte_offset(8) });
    print_i32(unsafe { *int_start.byte_add(16).byte_sub(4) });
    print_i32(unsafe { *int_start.byte_offset(0) });
    print_bool(unsafe { int_start.byte_add(8) } == unsafe { int_start.add(2) });
    print_usize(unsafe { int_start.byte_add(12) }.addr() - int_start.addr());
    // A mutable one, so the rebuilt pointer is a place and not just a value.
    let mut int_run_mut = [11i32, 22, 33, 44, 55];
    let int_start_mut = int_run_mut.as_mut_ptr();
    unsafe { *int_start_mut.byte_add(8) = 333 };
    print_i32(int_run_mut[2]);

    // The same arithmetic over a pointer to an aggregate, whose elements are objects.
    let pair_run = [Pair { first: 1, second: 2 }, Pair { first: 3, second: 4 }];
    let pair_start = pair_run.as_ptr();
    print_i32(unsafe { (*pair_start.byte_add(8)).first });
    print_bool(unsafe { pair_start.byte_add(8) } == unsafe { pair_start.add(1) });
}
