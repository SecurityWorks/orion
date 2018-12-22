// MIT License

// Copyright (c) 2018 brycx
// Based on the algorithm from https://github.com/floodyberry/poly1305-donna

// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:

// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.

// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

//! # About:
//! This implementation is based on [poly1305-donna](https://github.com/floodyberry/poly1305-donna)
//! by Andrew Moon.
//!
//! # Parameters:
//! - `data`: Data to be authenticated.
//! - `one_time_key`: One-time key used to authenticate.
//! - `expected`: The expected tag that needs to be verified.
//!
//! # Exceptions:
//! An exception will be thrown if:
//! - `finalize()` is called twice without a `reset()` in between.
//! - `update()` is called after `finalize()` without a `reset()` in between.
//! - The calculated tag does not match the expected when verifying.
//!
//! # Security:
//! - The one-time key should be generated using a CSPRNG. `OneTimeKey::generate()` can be used for this.
//!
//! # Recommendation:
//! - If you are unsure of wether to use HMAC or Poly1305, it is most often easier to just
//! use HMAC. See also [Cryptographic Right Answers](https://latacora.micro.blog/2018/04/03/cryptographic-right-answers.html).
//!
//! # Example:
//! ```
//! use orion::hazardous::mac::poly1305;
//!
//! let one_time_key = poly1305::OneTimeKey::generate().unwrap();
//! let msg = "Some message.";
//!
//! let mut poly1305_state = poly1305::init(&one_time_key);
//! poly1305_state.update(msg.as_bytes()).unwrap();
//! let tag = poly1305_state.finalize().unwrap();
//!
//! assert!(poly1305::verify(&tag, &one_time_key, msg.as_bytes()).unwrap());
//! ```

extern crate core;

use byteorder::{ByteOrder, LittleEndian};
use errors::*;
use hazardous::constants::{Poly1305Tag, POLY1305_BLOCKSIZE, POLY1305_KEYSIZE};

construct_secret_key!{
    /// A type to represent the `OneTimeKey` that Poly1305 uses for authentication.
    ///
    /// # Exceptions:
    /// An exception will be thrown if:
    /// - `slice` is not 32 bytes.
    /// - The `OsRng` fails to initialize or read from its source.
    (OneTimeKey, POLY1305_KEYSIZE)
}
construct_tag!{
    /// A type to represent the `Tag` that Poly1305 returns.
    ///
    /// # Exceptions:
    /// An exception will be thrown if:
    /// - `slice` is not 16 bytes.
    (Tag, POLY1305_BLOCKSIZE)
}

#[must_use]
/// Poly1305 as specified in the [RFC 8439](https://tools.ietf.org/html/rfc8439).
pub struct Poly1305 {
    a: [u32; 5],
    r: [u32; 5],
    s: [u32; 4],
    leftover: usize,
    buffer: [u8; POLY1305_BLOCKSIZE],
    is_finalized: bool,
}

impl Drop for Poly1305 {
    fn drop(&mut self) {
        use clear_on_drop::clear::Clear;
        self.a.clear();
        self.r.clear();
        self.s.clear();
        self.buffer.clear();
    }
}

impl core::fmt::Debug for Poly1305 {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(
            f,
            "Poly1305 {{ a: [***OMITTED***], r: [***OMITTED***], s: [***OMITTED***],
            leftover: ***OMITTED***, buffer: [***OMITTED***], is_finalized: {:?} }}",
            self.is_finalized
        )
    }
}

impl Poly1305 {
    #[inline(always)]
    #[cfg_attr(feature = "cargo-clippy", allow(clippy::unreadable_literal))]
    /// Initialize `Poly1305` struct for a given key.
    fn initialize(&mut self, key: &OneTimeKey) {
        // clamp(r)
        self.r[0] = (LittleEndian::read_u32(&key.unprotected_as_bytes()[0..4])) & 0x3ffffff;
        self.r[1] = (LittleEndian::read_u32(&key.unprotected_as_bytes()[3..7]) >> 2) & 0x3ffff03;
        self.r[2] = (LittleEndian::read_u32(&key.unprotected_as_bytes()[6..10]) >> 4) & 0x3ffc0ff;
        self.r[3] = (LittleEndian::read_u32(&key.unprotected_as_bytes()[9..13]) >> 6) & 0x3f03fff;
        self.r[4] = (LittleEndian::read_u32(&key.unprotected_as_bytes()[12..16]) >> 8) & 0x00fffff;

        self.s[0] = LittleEndian::read_u32(&key.unprotected_as_bytes()[16..20]);
        self.s[1] = LittleEndian::read_u32(&key.unprotected_as_bytes()[20..24]);
        self.s[2] = LittleEndian::read_u32(&key.unprotected_as_bytes()[24..28]);
        self.s[3] = LittleEndian::read_u32(&key.unprotected_as_bytes()[28..32]);
    }
    #[must_use]
    #[inline(never)]
    #[rustfmt::skip]
    #[cfg_attr(feature = "cargo-clippy", allow(clippy::cast_lossless))]
    #[cfg_attr(feature = "cargo-clippy", allow(clippy::identity_op))]
    #[cfg_attr(feature = "cargo-clippy", allow(clippy::unreadable_literal))]
    #[cfg_attr(feature = "cargo-clippy", allow(clippy::assign_op_pattern))]
    /// Process a datablock of `POLY1305_BLOCKSIZE` length.
    fn process_block(&mut self, data: &[u8]) -> Result<(), UnknownCryptoError> {
        if data.len() != POLY1305_BLOCKSIZE {
            return Err(UnknownCryptoError);
        }

        let hibit: u32 = if self.is_finalized {
            0
        } else {
            (1 << 24)
        };

        let r0: u32 = self.r[0];
        let r1: u32 = self.r[1];
        let r2: u32 = self.r[2];
        let r3: u32 = self.r[3];
        let r4: u32 = self.r[4];

        let s1: u32 = r1 * 5;
        let s2: u32 = r2 * 5;
        let s3: u32 = r3 * 5;
        let s4: u32 = r4 * 5;

        let mut h0: u32 = self.a[0];
        let mut h1: u32 = self.a[1];
        let mut h2: u32 = self.a[2];
        let mut h3: u32 = self.a[3];
        let mut h4: u32 = self.a[4];

        // h += m[i]
        h0 += (LittleEndian::read_u32(&data[0..4])) & 0x3ffffff;
        h1 += (LittleEndian::read_u32(&data[3..7]) >> 2) & 0x3ffffff;
        h2 += (LittleEndian::read_u32(&data[6..10]) >> 4) & 0x3ffffff;
        h3 += (LittleEndian::read_u32(&data[9..13]) >> 6) & 0x3ffffff;
        h4 += (LittleEndian::read_u32(&data[12..16]) >> 8) | hibit;

        // h *= r
        let d0: u64 =
            (h0 as u64 * r0 as u64) +
            (h1 as u64 * s4 as u64) +
            (h2 as u64 * s3 as u64) +
            (h3 as u64 * s2 as u64) +
            (h4 as u64 * s1 as u64);
        let mut d1: u64 =
            (h0 as u64 * r1 as u64) +
            (h1 as u64 * r0 as u64) +
            (h2 as u64 * s4 as u64) +
            (h3 as u64 * s3 as u64) +
            (h4 as u64 * s2 as u64);
        let mut d2: u64 =
            (h0 as u64 * r2 as u64) +
            (h1 as u64 * r1 as u64) +
            (h2 as u64 * r0 as u64) +
            (h3 as u64 * s4 as u64) +
            (h4 as u64 * s3 as u64);
        let mut d3: u64 =
            (h0 as u64 * r3 as u64) +
            (h1 as u64 * r2 as u64) +
            (h2 as u64 * r1 as u64) +
            (h3 as u64 * r0 as u64) +
            (h4 as u64 * s4 as u64);
        let mut d4: u64 =
            (h0 as u64 * r4 as u64) +
            (h1 as u64 * r3 as u64) +
            (h2 as u64 * r2 as u64) +
            (h3 as u64 * r1 as u64) +
            (h4 as u64 * r0 as u64);

        // (partial) h %= p
        let mut c: u32 = (d0 >> 26) as u32; h0 = (d0 & 0x3ffffff) as u32;
        d1 += c as u64; c = (d1 >> 26) as u32; h1 = (d1 & 0x3ffffff) as u32;
        d2 += c as u64; c = (d2 >> 26) as u32; h2 = (d2 & 0x3ffffff) as u32;
        d3 += c as u64; c = (d3 >> 26) as u32; h3 = (d3 & 0x3ffffff) as u32;
        d4 += c as u64; c = (d4 >> 26) as u32; h4 = (d4 & 0x3ffffff) as u32;
        h0 += c * 5; c = h0 >> 26; h0 = h0 & 0x3ffffff;
        h1 += c;

        self.a[0] = h0;
        self.a[1] = h1;
        self.a[2] = h2;
        self.a[3] = h3;
        self.a[4] = h4;

        Ok(())
    }
    #[inline(never)]
    #[rustfmt::skip]
    #[cfg_attr(feature = "cargo-clippy", allow(clippy::cast_lossless))]
    #[cfg_attr(feature = "cargo-clippy", allow(clippy::identity_op))]
    #[cfg_attr(feature = "cargo-clippy", allow(clippy::unreadable_literal))]
    #[cfg_attr(feature = "cargo-clippy", allow(clippy::assign_op_pattern))]
    /// Remaining processing after all data blocks have been processed.
    fn process_end_of_stream(&mut self) {
        // full carry h
        let mut h0: u32 = self.a[0];
        let mut h1: u32 = self.a[1];
        let mut h2: u32 = self.a[2];
        let mut h3: u32 = self.a[3];
        let mut h4: u32 = self.a[4];

        let mut c: u32 = h1 >> 26; h1 = h1 & 0x3ffffff;
        h2 += c; c = h2 >> 26; h2 = h2 & 0x3ffffff;
        h3 += c; c = h3 >> 26; h3 = h3 & 0x3ffffff;
        h4 += c; c = h4 >> 26; h4 = h4 & 0x3ffffff;
        h0 += c * 5; c = h0 >> 26; h0 = h0 & 0x3ffffff;
        h1 += c;

        // compute h + -p
        let mut g0: u32 = h0.wrapping_add(5); c = g0 >> 26; g0 &= 0x3ffffff;
        let mut g1: u32 = h1.wrapping_add(c); c = g1 >> 26; g1 &= 0x3ffffff;
        let mut g2: u32 = h2.wrapping_add(c); c = g2 >> 26; g2 &= 0x3ffffff;
        let mut g3: u32 = h3.wrapping_add(c); c = g3 >> 26; g3 &= 0x3ffffff;
        let mut g4: u32 = h4.wrapping_add(c).wrapping_sub(1 << 26);

        // select h if h < p, or h + -p if h >= p
        let mut mask = (g4 >> (32 - 1)).wrapping_sub(1);
        g0 &= mask;
        g1 &= mask;
        g2 &= mask;
        g3 &= mask;
        g4 &= mask;
        mask = !mask;
        h0 = (h0 & mask) | g0;
    	h1 = (h1 & mask) | g1;
    	h2 = (h2 & mask) | g2;
    	h3 = (h3 & mask) | g3;
    	h4 = (h4 & mask) | g4;

        // h = h % (2^128)
        h0 = ((h0) | (h1 << 26)) & 0xffffffff;
        h1 = ((h1 >> 6) | (h2 << 20)) & 0xffffffff;
        h2 = ((h2 >> 12) | (h3 << 14)) & 0xffffffff;
        h3 = ((h3 >> 18) | (h4 << 8)) & 0xffffffff;

        // mac = (h + pad) % (2^128)
        let mut f: u64 = (h0 as u64) + (self.s[0] as u64); h0 = f as u32;
        f = (h1 as u64) + (self.s[1] as u64) + (f >> 32); h1 = f as u32;
        f = (h2 as u64) + (self.s[2] as u64) + (f >> 32); h2 = f as u32;
        f = (h3 as u64) + (self.s[3] as u64) + (f >> 32); h3 = f as u32;

        // Set self.a to MAC result
        self.a[0] = h0;
        self.a[1] = h1;
        self.a[2] = h2;
        self.a[3] = h3;
    }

	#[inline(always)]
	/// Reset to `init()` state.
	pub fn reset(&mut self) {
		self.a = [0u32; 5];
		self.leftover = 0;
		self.is_finalized = false;
		self.buffer = [0u8; POLY1305_BLOCKSIZE];
	}

	#[must_use]
	#[inline(always)]
	/// Update state with a `data`. This can be called multiple times.
	pub fn update(&mut self, data: &[u8]) -> Result<(), FinalizationCryptoError> {
		if self.is_finalized {
			return Err(FinalizationCryptoError);
		}

		let mut bytes = data;

		if self.leftover != 0 {
			let mut want = POLY1305_BLOCKSIZE - self.leftover;
			if want > bytes.len() {
				want = bytes.len();
			}

			for (idx, itm) in bytes.iter().enumerate().take(want) {
				self.buffer[self.leftover + idx] = *itm;
			}
			// Reduce by slice
			bytes = &bytes[want..];
			self.leftover += want;

			if self.leftover < POLY1305_BLOCKSIZE {
				return Ok(());
			}

			let tmp = self.buffer;
			self.process_block(&tmp).unwrap();
			self.leftover = 0;
		}

		while bytes.len() >= POLY1305_BLOCKSIZE {
			self.process_block(&bytes[0..POLY1305_BLOCKSIZE]).unwrap();
			// Reduce by slice
			bytes = &bytes[POLY1305_BLOCKSIZE..];
		}

		self.buffer[..bytes.len()].copy_from_slice(&bytes);
		self.leftover = bytes.len();

		Ok(())
	}

	#[must_use]
	#[inline(always)]
	/// Return a Poly1305 tag.
	pub fn finalize(&mut self) -> Result<Tag, FinalizationCryptoError> {
		if self.is_finalized {
			return Err(FinalizationCryptoError);
		}

		self.is_finalized = true;

		let mut local_buffer: Poly1305Tag = self.buffer;

		if self.leftover != 0 {
			local_buffer[self.leftover] = 1;
			// Pad the last block with zeroes before processing it
			for buf_itm in local_buffer
				.iter_mut()
				.take(POLY1305_BLOCKSIZE)
				.skip(self.leftover + 1)
			{
				*buf_itm = 0u8;
			}

			self.process_block(&local_buffer).unwrap();
		}
		// Get tag
		self.process_end_of_stream();
		LittleEndian::write_u32_into(&self.a[0..4], &mut local_buffer);

		Ok(Tag::from_slice(&local_buffer).unwrap())
	}
}

#[must_use]
/// Initialize a `Poly1305` struct with a given one-time key.
pub fn init(one_time_key: &OneTimeKey) -> Poly1305 {
    let mut poly_1305_state = Poly1305 {
        a: [0u32; 5],
        r: [0u32; 5],
        s: [0u32; 4],
        leftover: 0,
        buffer: [0u8; POLY1305_BLOCKSIZE],
        is_finalized: false,
    };

    poly_1305_state.initialize(one_time_key);

    poly_1305_state
}

#[must_use]
/// One-shot function for generating a Poly1305 tag of `data`.
pub fn poly1305(one_time_key: &OneTimeKey, data: &[u8]) -> Result<Tag, UnknownCryptoError> {
    let mut poly_1305_state = init(one_time_key);
    poly_1305_state.update(data).unwrap();

    Ok(poly_1305_state.finalize().unwrap())
}

#[must_use]
/// Verify a Poly1305 tag in constant time.
pub fn verify(
    expected: &Tag,
    one_time_key: &OneTimeKey,
    data: &[u8],
) -> Result<bool, ValidationCryptoError> {
    if &poly1305(one_time_key, data)? == expected {
        Ok(true)
    } else {
        Err(ValidationCryptoError)
    }
}

#[test]
fn test_wrong_key_len() {
    assert!(OneTimeKey::from_slice(&[0u8; 31]).is_err());
    assert!(OneTimeKey::from_slice(&[0u8; 33]).is_err());
    assert!(OneTimeKey::from_slice(&[0u8; 32]).is_ok());
}

#[test]
fn test_poly1305_oneshot_ok() {
    assert!(poly1305(&OneTimeKey::from_slice(&[0u8; 32]).unwrap(), &[0u8; 16]).is_ok());
}

#[test]
fn test_poly1305_verify_ok() {
    let tag = poly1305(&OneTimeKey::from_slice(&[0u8; 32]).unwrap(), &[0u8; 16]).unwrap();
    verify(
        &tag,
        &OneTimeKey::from_slice(&[0u8; 32]).unwrap(),
        &[0u8; 16],
    ).unwrap();
}

#[test]
fn test_poly1305_verify_err() {
    let mut tag = poly1305(&OneTimeKey::from_slice(&[0u8; 32]).unwrap(), &[0u8; 16]).unwrap();
    tag.value[0] ^= 1;
    assert!(verify(
        &tag,
        &OneTimeKey::from_slice(&[0u8; 32]).unwrap(),
        &[0u8; 16],
    ).is_err());
}

#[test]
fn test_bad_key_err_less() {
    assert!(OneTimeKey::from_slice(&[0u8; 31]).is_err());
}

#[test]
fn test_poly1305_oneshot_bad_key_err_greater() {
    assert!(OneTimeKey::from_slice(&[0u8; 33]).is_err());
}

#[test]
fn double_finalize_err() {
    let mut poly1305_state = init(&OneTimeKey::from_slice(&[0u8; 32]).unwrap());

    poly1305_state.update(&[0u8; 16]).unwrap();
    let _ = poly1305_state.finalize().unwrap();
    assert!(poly1305_state.finalize().is_err());
}

#[test]
fn double_finalize_with_reset_ok() {
    let mut poly1305_state = init(&OneTimeKey::from_slice(&[0u8; 32]).unwrap());

    poly1305_state.update(&[0u8; 16]).unwrap();
    let _ = poly1305_state.finalize().unwrap();
    poly1305_state.reset();
    poly1305_state.update(&[0u8; 16]).unwrap();
    let _ = poly1305_state.finalize().unwrap();
}

#[test]
fn double_finalize_with_reset_no_update_ok() {
    let mut poly1305_state = init(&OneTimeKey::from_slice(&[0u8; 32]).unwrap());

    poly1305_state.update(&[0u8; 16]).unwrap();
    let _ = poly1305_state.finalize().unwrap();
    poly1305_state.reset();
    let _ = poly1305_state.finalize().unwrap();
}

#[test]
fn update_after_finalize_err() {
    let mut poly1305_state = init(&OneTimeKey::from_slice(&[0u8; 32]).unwrap());

    poly1305_state.update(&[0u8; 16]).unwrap();
    let _ = poly1305_state.finalize().unwrap();
    assert!(poly1305_state.update(&[0u8; 16]).is_err());
}

#[test]
fn update_after_finalize_with_reset_ok() {
    let mut poly1305_state = init(&OneTimeKey::from_slice(&[0u8; 32]).unwrap());

    poly1305_state.update(&[0u8; 16]).unwrap();
    let expected = poly1305_state.finalize().unwrap();
    poly1305_state.reset();
    poly1305_state.update(&[0u8; 16]).unwrap();
    assert!(&expected == &poly1305_state.finalize().unwrap());
}

#[test]
fn double_reset_ok() {
    let mut poly1305_state = init(&OneTimeKey::from_slice(&[0u8; 32]).unwrap());

    poly1305_state.update(&[0u8; 16]).unwrap();
    let _ = poly1305_state.finalize().unwrap();
    poly1305_state.reset();
    poly1305_state.reset();
}

#[test]
fn reset_after_update_correct_resets() {
	let secret_key = OneTimeKey::from_slice(&[0u8; 32]).unwrap();

	let state_1 = init(&secret_key);

	let mut state_2 = init(&secret_key);
	state_2.update(b"Tests").unwrap();
	state_2.reset();

	assert_eq!(state_1.a, state_2.a);
	assert_eq!(state_1.r, state_2.r);
	assert_eq!(state_1.s, state_2.s);
	assert_eq!(state_1.leftover, state_2.leftover);
	assert_eq!(state_1.buffer[..], state_2.buffer[..]);
	assert_eq!(state_1.is_finalized, state_2.is_finalized);
}

#[test]
fn reset_after_update_correct_resets_and_verify() {
	let secret_key = OneTimeKey::from_slice(&[0u8; 32]).unwrap();

	let mut state_1 = init(&secret_key);
	state_1.update(b"Tests").unwrap();
	let d1 = state_1.finalize().unwrap();

	let mut state_2 = init(&secret_key);
	state_2.update(b"Tests").unwrap();
	state_2.reset();
	state_2.update(b"Tests").unwrap();
	let d2 = state_2.finalize().unwrap();

	assert_eq!(d1, d2);
}
