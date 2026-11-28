use bytes::{Buf, BufMut, Bytes, BytesMut};

#[test]
fn bytes_mut_freeze_slice_and_drop() {
    let mut buf = BytesMut::with_capacity(64);
    buf.put_u32(0x1234_5678);
    buf.put_u16_le(0xABCD);
    buf.put_i64(-0x0102_0304_0506_0708);
    buf.put_slice(b"hello mires");

    let mut frozen: Bytes = buf.freeze();
    let shared = frozen.clone();

    assert_eq!(frozen.get_u32(), 0x1234_5678);
    assert_eq!(frozen.get_u16_le(), 0xABCD);
    assert_eq!(frozen.get_i64(), -0x0102_0304_0506_0708);
    assert_eq!(&frozen[..], b"hello mires");

    let sub = shared.slice(14..19);
    assert_eq!(&sub[..], b"hello");
    let tail = shared.slice(20..);
    assert_eq!(&tail[..], b"mires");

    drop(frozen);
    drop(sub);
    drop(tail);
    drop(shared);
}

#[test]
fn bytes_mut_split_and_copy_from_slice() {
    let mut b = BytesMut::from(&b"alpha-beta-gamma"[..]);
    let first = b.split_to(6);
    assert_eq!(&first[..], b"alpha-");
    let last = b.split_off(5);
    assert_eq!(&b[..], b"beta-");
    assert_eq!(&last[..], b"gamma");

    let static_b = Bytes::from_static(b"static-payload");
    assert_eq!(static_b.len(), 14);
    assert_eq!(&static_b.slice(7..)[..], b"payload");
}
