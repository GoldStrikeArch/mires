//! Synchronous streaming classes (`Deflate`, `Gzip`, `Zlib`, `Inflate`, `Gunzip`, `Unzlib`,
//! `Decompress`), matching fflate 0.8.3 chunk boundaries, `final` flags, and errors bit-for-bit.

use alloc::vec;
use alloc::vec::Vec;

use crate::deflate::{dopt, shft, CompressOptions, DeflateState};
use crate::framing::{detect_format, gzh, gzhl, gzs, wbytes, zlh, zls, Adler32, Crc32, FlateError};
use crate::inflate::{inflt, DecompressOptions, InflateState};

/// Framed output sink matching CONTRACT.md: `[u32le(chunk.len()), final ? 1 : 0, ...chunk]`.
pub struct StreamSink {
    pub has_handler: bool,
    pub out: Vec<u8>,
}

impl StreamSink {
    pub fn new(has_handler: bool) -> Self {
        Self {
            has_handler,
            out: Vec::new(),
        }
    }

    pub fn emit(&mut self, chunk: &[u8], is_final: bool) {
        let len = chunk.len() as u32;
        self.out.reserve(5 + chunk.len());
        self.out.push((len & 0xFF) as u8);
        self.out.push(((len >> 8) & 0xFF) as u8);
        self.out.push(((len >> 16) & 0xFF) as u8);
        self.out.push(((len >> 24) & 0xFF) as u8);
        self.out.push(if is_final { 1 } else { 0 });
        self.out.extend_from_slice(chunk);
    }
}

enum CompressFrame {
    Raw,
    Gzip { crc: Crc32, len: usize, v: bool },
    Zlib { adler: Adler32, v: bool },
}

/// Shared implementation of `Deflate`, `Gzip`, and `Zlib` streams.
pub struct CompressStream {
    o: CompressOptions,
    s: DeflateState,
    b: Vec<u8>,
    frame: CompressFrame,
}

impl CompressStream {
    pub fn new_deflate(opts: CompressOptions) -> Self {
        Self::new_inner(opts, CompressFrame::Raw)
    }

    pub fn new_gzip(opts: CompressOptions) -> Self {
        Self::new_inner(
            opts,
            CompressFrame::Gzip {
                crc: Crc32::new(),
                len: 0,
                v: true,
            },
        )
    }

    pub fn new_zlib(opts: CompressOptions) -> Self {
        Self::new_inner(
            opts,
            CompressFrame::Zlib {
                adler: Adler32::new(),
                v: true,
            },
        )
    }

    fn new_inner(opts: CompressOptions, frame: CompressFrame) -> Self {
        let mut s = DeflateState::streaming();
        let mut b = vec![0u8; 98304];
        if let Some(ref full_dict) = opts.dictionary {
            let dlen = full_dict.len();
            let dict = if dlen > 32768 {
                &full_dict[dlen - 32768..]
            } else {
                full_dict.as_slice()
            };
            let start = 32768 - dict.len();
            b[start..32768].copy_from_slice(dict);
            s.i = start;
        }
        Self { o: opts, s, b, frame }
    }

    fn emit_block(&mut self, is_final: bool, sink: &mut StreamSink) {
        match &mut self.frame {
            CompressFrame::Raw => {
                let raw = dopt(&self.b, &self.o, 0, 0, Some(&mut self.s));
                sink.emit(&raw, is_final);
            }
            CompressFrame::Gzip { crc, len, v } => {
                let pre = if *v { gzhl(&self.o) } else { 0 };
                let post = if is_final { 8 } else { 0 };
                let mut raw = dopt(&self.b, &self.o, pre, post, Some(&mut self.s));
                if *v {
                    gzh(&mut raw, &self.o);
                    *v = false;
                }
                if is_final {
                    let rlen = raw.len();
                    wbytes(&mut raw, rlen - 8, crc.digest());
                    wbytes(&mut raw, rlen - 4, *len as u32);
                }
                sink.emit(&raw, is_final);
            }
            CompressFrame::Zlib { adler, v } => {
                let pre = if *v {
                    if self.o.dictionary.is_some() { 6 } else { 2 }
                } else {
                    0
                };
                let post = if is_final { 4 } else { 0 };
                let mut raw = dopt(&self.b, &self.o, pre, post, Some(&mut self.s));
                if *v {
                    zlh(&mut raw, &self.o);
                    *v = false;
                }
                if is_final {
                    let rlen = raw.len();
                    wbytes(&mut raw, rlen - 4, adler.digest_for_wbytes());
                }
                sink.emit(&raw, is_final);
            }
        }
    }

    pub fn push(
        &mut self,
        chunk: &[u8],
        is_final: bool,
        sink: &mut StreamSink,
    ) -> Result<(), FlateError> {
        match &mut self.frame {
            CompressFrame::Raw => {}
            CompressFrame::Gzip { crc, len, .. } => {
                crc.push(chunk);
                *len += chunk.len();
            }
            CompressFrame::Zlib { adler, .. } => {
                adler.push(chunk);
            }
        }
        if !sink.has_handler {
            return Err(FlateError::Code(5));
        }
        if self.s.l != 0 {
            return Err(FlateError::Code(4));
        }
        let end_len = chunk.len() + self.s.z;
        if end_len > self.b.len() {
            if end_len > 2 * self.b.len() - 32768 {
                let new_len = end_len & !32767usize;
                let mut new_buf = vec![0u8; new_len];
                new_buf[..self.s.z].copy_from_slice(&self.b[..self.s.z]);
                self.b = new_buf;
            }
            let split = self.b.len() - self.s.z;
            self.b[self.s.z..].copy_from_slice(&chunk[..split]);
            self.s.z = self.b.len();
            self.emit_block(false, sink);
            let blen = self.b.len();
            self.b.copy_within(blen - 32768..blen, 0);
            let rest = &chunk[split..];
            self.b[32768..32768 + rest.len()].copy_from_slice(rest);
            self.s.z = rest.len() + 32768;
            self.s.i = 32766;
            self.s.w = 32768;
        } else {
            self.b[self.s.z..self.s.z + chunk.len()].copy_from_slice(chunk);
            self.s.z += chunk.len();
        }
        self.s.l = if is_final { 1 } else { 0 };
        if self.s.z > self.s.w + 8191 || is_final {
            self.emit_block(is_final, sink);
            self.s.w = self.s.i;
            self.s.i = self.s.i.saturating_sub(2);
        }
        if is_final {
            self.s = DeflateState::streaming();
            self.s.z = 0;
            self.o = CompressOptions::empty();
            self.b.clear();
        }
        Ok(())
    }

    pub fn flush(&mut self, sync: bool, sink: &mut StreamSink) -> Result<(), FlateError> {
        if !sink.has_handler {
            return Err(FlateError::Code(5));
        }
        if self.s.l != 0 {
            return Err(FlateError::Code(4));
        }
        self.emit_block(false, sink);
        self.s.w = self.s.i;
        self.s.i = self.s.i.saturating_sub(2);
        if sync {
            // Mirror fflate 0.8.3 lines 1104-1109 verbatim.
            let mut c = [0u8; 6];
            c[0] = (self.s.r >> 3) as u8;
            let o = shft(self.s.r + 2);
            if o < 6 {
                c[o] = 0;
            }
            if o + 1 < 6 {
                c[o + 1] = 0;
            }
            if o + 2 < 6 {
                c[o + 2] = 255;
            }
            if o + 3 < 6 {
                c[o + 3] = 255;
            }
            let ep_bytes = if o + 4 < 6 { o + 4 } else { 6 };
            self.s.r = 0;
            sink.emit(&c[..ep_bytes], false);
        }
        Ok(())
    }
}

/// Streaming raw DEFLATE decompressor (fflate `Inflate`).
pub struct InflateStream {
    s: InflateState,
    o: Vec<u8>,
    p: Vec<u8>,
    d: bool,
}

impl InflateStream {
    pub fn new(opts: &DecompressOptions) -> Self {
        let mut o = vec![0u8; 32768];
        let mut initial_b = 0;
        if let Some(ref full_dict) = opts.dictionary {
            let dlen = full_dict.len();
            let dict = if dlen > 32768 {
                &full_dict[dlen - 32768..]
            } else {
                full_dict.as_slice()
            };
            o[..dict.len()].copy_from_slice(dict);
            initial_b = dict.len();
        }
        Self {
            s: InflateState::streaming(initial_b),
            o,
            p: Vec::new(),
            d: false,
        }
    }

    fn e(&mut self, chunk: &[u8], sink: &StreamSink) -> Result<(), FlateError> {
        if !sink.has_handler {
            return Err(FlateError::Code(5));
        }
        if self.d {
            return Err(FlateError::Code(4));
        }
        if self.p.is_empty() {
            if !chunk.is_empty() {
                self.p.extend_from_slice(chunk);
            }
        } else if !chunk.is_empty() {
            self.p.extend_from_slice(chunk);
        }
        Ok(())
    }

    fn c(&mut self, is_final: bool, sink: &mut StreamSink) -> Result<(), FlateError> {
        self.d = is_final;
        self.s.i = if is_final { 1 } else { 0 };
        let bts = self.s.b;
        let buf = core::mem::take(&mut self.o);
        let dt = inflt(&self.p, &mut self.s, Some(buf), None)?;
        let end = if self.s.b < dt.len() { self.s.b } else { dt.len() };
        let start = if bts < end { bts } else { end };
        sink.emit(&dt[start..end], self.d);

        let keep_start = self.s.b.saturating_sub(32768);
        let keep_start = if keep_start < dt.len() { keep_start } else { dt.len() };
        self.o = dt[keep_start..].to_vec();
        self.s.b = self.o.len();

        let byte_pos = self.s.p >> 3;
        if byte_pos >= self.p.len() {
            self.p.clear();
        } else if byte_pos > 0 {
            self.p = self.p[byte_pos..].to_vec();
        }
        self.s.p &= 7;
        Ok(())
    }

    pub fn push(
        &mut self,
        chunk: &[u8],
        is_final: bool,
        sink: &mut StreamSink,
    ) -> Result<(), FlateError> {
        self.e(chunk, sink)?;
        self.c(is_final, sink)
    }
}

/// Streaming single- or multi-member GZIP decompressor (fflate `Gunzip`).
pub struct GunzipStream {
    inner: InflateStream,
    v: usize,
}

impl GunzipStream {
    pub fn new(opts: &DecompressOptions) -> Self {
        Self {
            inner: InflateStream::new(opts),
            v: 1,
        }
    }

    pub fn push(
        &mut self,
        chunk: &[u8],
        is_final: bool,
        sink: &mut StreamSink,
    ) -> Result<(), FlateError> {
        self.inner.e(chunk, sink)?;
        if self.v != 0 {
            let p_start = self.v - 1;
            let p_slice: &[u8] = if p_start < self.inner.p.len() {
                &self.inner.p[p_start..]
            } else {
                &[]
            };
            let s = if p_slice.len() > 3 {
                gzs(p_slice)?
            } else {
                4
            };
            if s > p_slice.len() {
                if !is_final {
                    return Ok(());
                }
            }
            let next_p = if s < p_slice.len() {
                p_slice[s..].to_vec()
            } else {
                Vec::new()
            };
            self.inner.p = next_p;
            self.v = 0;
        }
        self.inner.c(false, sink)?;
        if self.inner.s.f != 0 && self.inner.s.l.is_none() {
            self.v = shft(self.inner.s.p) + 9;
            self.inner.s = InflateState::streaming(0);
            self.inner.o.clear();
            self.push(&[], is_final, sink)?;
        } else if is_final {
            self.inner.c(true, sink)?;
        }
        Ok(())
    }
}

/// Streaming Zlib decompressor (fflate `Unzlib`).
pub struct UnzlibStream {
    inner: InflateStream,
    v: u8,
}

impl UnzlibStream {
    pub fn new(opts: &DecompressOptions) -> Self {
        let v = if opts.dictionary.is_some() { 2 } else { 1 };
        Self {
            inner: InflateStream::new(opts),
            v,
        }
    }

    pub fn push(
        &mut self,
        chunk: &[u8],
        is_final: bool,
        sink: &mut StreamSink,
    ) -> Result<(), FlateError> {
        self.inner.e(chunk, sink)?;
        if self.v != 0 {
            if self.inner.p.len() < 6 && !is_final {
                return Ok(());
            }
            let start = zls(&self.inner.p, self.v > 1)?;
            self.inner.p = if start < self.inner.p.len() {
                self.inner.p[start..].to_vec()
            } else {
                Vec::new()
            };
            self.v = 0;
        }
        if is_final {
            if self.inner.p.len() < 4 {
                return Err(FlateError::Code(6));
            }
            let new_len = self.inner.p.len() - 4;
            self.inner.p.truncate(new_len);
        }
        self.inner.c(is_final, sink)
    }
}

enum DecompressInner {
    Gunzip(GunzipStream),
    Inflate(InflateStream),
    Unzlib(UnzlibStream),
}

/// Streaming auto-detecting decompressor (fflate `Decompress`).
pub struct DecompressStream {
    opts: DecompressOptions,
    p: Vec<u8>,
    inner: Option<DecompressInner>,
}

impl DecompressStream {
    pub fn new(opts: DecompressOptions) -> Self {
        Self {
            opts,
            p: Vec::new(),
            inner: None,
        }
    }

    pub fn push(
        &mut self,
        chunk: &[u8],
        is_final: bool,
        sink: &mut StreamSink,
    ) -> Result<(), FlateError> {
        if !sink.has_handler {
            return Err(FlateError::Code(5));
        }
        if self.inner.is_none() {
            if self.p.is_empty() {
                self.p.extend_from_slice(chunk);
            }
            // Note: matches fflate 0.8.3 lines 1564-1567 (see CONTRACT.md Parity domain).
            if self.p.len() > 2 {
                let mut stream = match detect_format(&self.p) {
                    0 => DecompressInner::Gunzip(GunzipStream::new(&self.opts)),
                    1 => DecompressInner::Inflate(InflateStream::new(&self.opts)),
                    _ => DecompressInner::Unzlib(UnzlibStream::new(&self.opts)),
                };
                let initial = core::mem::take(&mut self.p);
                match &mut stream {
                    DecompressInner::Gunzip(g) => g.push(&initial, is_final, sink)?,
                    DecompressInner::Inflate(i) => i.push(&initial, is_final, sink)?,
                    DecompressInner::Unzlib(u) => u.push(&initial, is_final, sink)?,
                }
                self.inner = Some(stream);
            }
            Ok(())
        } else {
            match self.inner.as_mut() {
                Some(DecompressInner::Gunzip(g)) => g.push(chunk, is_final, sink),
                Some(DecompressInner::Inflate(i)) => i.push(chunk, is_final, sink),
                Some(DecompressInner::Unzlib(u)) => u.push(chunk, is_final, sink),
                None => Ok(()),
            }
        }
    }
}
