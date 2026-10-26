//! BitTorrent MSE/PE negotiation and stream adapters (legacy interoperability).
//! The wire format mandates its 768-bit DH group, SHA-1 and RC4-drop1024.
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use crypto_bigint::{
    Encoding, U256, U768,
    modular::runtime_mod::{DynResidue, DynResidueParams},
};
use rand::{TryRng, rngs::SysRng};
use rc4::{KeyInit, Rc4, StreamCipher, consts::U20};
use serde::{Deserialize, Serialize};
use sha1w::{ISha1, Sha1};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use zeroize::Zeroizing;

pub const PLAIN_PREFIX: &[u8; 20] = b"\x13BitTorrent protocol";
const MAX_PAD: usize = 512;
const PRIME: U768 = U768::from_be_hex(
    "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A63A36210000000000090563",
);
const PARAMS: DynResidueParams<{ U768::LIMBS }> = DynResidueParams::new(&PRIME);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncryptionPolicy {
    Disabled,
    /// Accept MSE, try plaintext first, then MSE on a fresh connection.
    #[default]
    Allow,
    /// Try MSE first, allowing a fresh plaintext connection if it fails.
    Prefer,
    /// Require RC4 payload encryption; neither bare nor header-only is accepted.
    Require,
}

impl EncryptionPolicy {
    pub(crate) fn attempts(self) -> &'static [bool] {
        match self {
            Self::Disabled => &[false],
            Self::Allow => &[false, true],
            Self::Prefer => &[true, false],
            Self::Require => &[true],
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("MSE I/O: {0}")]
    Io(#[from] io::Error),
    #[error("MSE: invalid DH public key")]
    PublicKey,
    #[error("MSE: synchronization marker missing or padding exceeds 512 bytes")]
    Sync,
    #[error("MSE: unknown torrent stream key")]
    UnknownKey,
    #[error("MSE: invalid verification constant")]
    Verification,
    #[error("MSE: invalid padding length")]
    Padding,
    #[error("MSE: cipher selection violates encryption policy")]
    Policy,
    #[error("MSE: operating system randomness unavailable")]
    Random,
}

pub fn stream_key_hash(key: &[u8; 20]) -> [u8; 20] {
    hash(&[b"req2", key])
}
fn hash(parts: &[&[u8]]) -> [u8; 20] {
    let mut state = Sha1::new();
    for part in parts {
        state.update(part);
    }
    state.finish()
}
fn random(bytes: &mut [u8]) -> Result<(), Error> {
    SysRng.try_fill_bytes(bytes).map_err(|_| Error::Random)
}
fn padding() -> Result<Vec<u8>, Error> {
    let mut length = [0; 2];
    random(&mut length)?;
    let mut data = vec![0; usize::from(u16::from_be_bytes(length)) % (MAX_PAD + 1)];
    random(&mut data)?;
    Ok(data)
}

struct Exchange {
    secret: Zeroizing<U256>,
}
impl Exchange {
    fn new() -> Result<Self, Error> {
        let mut bytes = Zeroizing::new([0; 32]);
        random(bytes.as_mut())?;
        // A nonzero fixed-width exponent; execution does not depend on its bit length.
        bytes[0] |= 0x80;
        Ok(Self {
            secret: Zeroizing::new(U256::from_be_slice(bytes.as_ref())),
        })
    }
    fn public(&self) -> [u8; 96] {
        DynResidue::new(&U768::from(2u8), PARAMS)
            .pow(&*self.secret)
            .retrieve()
            .to_be_bytes()
    }
    fn shared(&self, key: &[u8; 96]) -> Result<Zeroizing<[u8; 96]>, Error> {
        let public = U768::from_be_slice(key);
        if public < U768::from(2u8) || public >= PRIME.wrapping_sub(&U768::ONE) {
            return Err(Error::PublicKey);
        }
        let shared = Zeroizing::new(
            DynResidue::new(&public, PARAMS)
                .pow(&*self.secret)
                .retrieve(),
        );
        Ok(Zeroizing::new(shared.to_be_bytes()))
    }
}

struct Cipher(Rc4<U20>);
impl Cipher {
    fn new(label: &[u8], shared: &[u8], key: &[u8; 20]) -> Self {
        let digest = Zeroizing::new(hash(&[label, shared, key]));
        let mut cipher =
            Self(Rc4::<U20>::new_from_slice(digest.as_ref()).expect("SHA-1 key has 20 bytes"));
        let mut discard = Zeroizing::new([0; 1024]);
        cipher.apply(discard.as_mut());
        cipher
    }
    fn apply(&mut self, data: &mut [u8]) {
        self.0.apply_keystream(data);
    }
}

async fn scan<R: AsyncRead + Unpin>(read: &mut R, marker: &[u8]) -> Result<(), Error> {
    let mut window = vec![0; marker.len()];
    read.read_exact(&mut window).await?;
    for skip in 0..=MAX_PAD {
        if window == marker {
            return Ok(());
        }
        if skip == MAX_PAD {
            break;
        }
        window.rotate_left(1);
        read.read_exact(&mut window[marker.len() - 1..]).await?;
    }
    Err(Error::Sync)
}
async fn encrypted_read<R: AsyncRead + Unpin>(
    read: &mut R,
    cipher: &mut Cipher,
    length: usize,
) -> Result<Vec<u8>, Error> {
    let mut data = vec![0; length];
    read.read_exact(&mut data).await?;
    cipher.apply(&mut data);
    Ok(data)
}
async fn send<W: AsyncWrite + Unpin>(write: &mut W, data: &[u8]) -> Result<(), Error> {
    write.write_all(data).await?;
    write.flush().await?;
    Ok(())
}

pub struct Negotiated<R, W> {
    pub read: EncryptedRead<R>,
    pub write: EncryptedWrite<W>,
    pub encrypted: bool,
    pub info_hash: Option<[u8; 20]>,
}
fn negotiated<R, W>(
    read: R,
    write: W,
    key: Option<[u8; 20]>,
    rx: Option<Cipher>,
    tx: Option<Cipher>,
    prefix: Vec<u8>,
) -> Negotiated<R, W> {
    Negotiated {
        encrypted: rx.is_some(),
        info_hash: key,
        read: EncryptedRead {
            inner: read,
            cipher: rx,
            prefix,
            position: 0,
        },
        write: EncryptedWrite {
            inner: write,
            cipher: tx,
            pending: Vec::new(),
            position: 0,
        },
    }
}

/// Negotiate as initiator. `initial` is encrypted as IA even for header-only PE.
pub async fn outgoing<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut read: R,
    mut write: W,
    key: [u8; 20],
    initial: &[u8],
    policy: EncryptionPolicy,
) -> Result<Negotiated<R, W>, Error> {
    if policy == EncryptionPolicy::Disabled || initial.len() > u16::MAX as usize {
        return Err(Error::Policy);
    }
    let exchange = Exchange::new()?;
    let mut hello = exchange.public().to_vec();
    hello.extend(padding()?);
    send(&mut write, &hello).await?;
    let mut remote = [0; 96];
    read.read_exact(&mut remote).await?;
    let shared = exchange.shared(&remote)?;
    let mut tx = Cipher::new(b"keyA", shared.as_ref(), &key);
    let mut rx = Cipher::new(b"keyB", shared.as_ref(), &key);
    let mut request = hash(&[b"req1", shared.as_ref()]).to_vec();
    let mask = hash(&[b"req3", shared.as_ref()]);
    request.extend(stream_key_hash(&key).iter().zip(mask).map(|(a, b)| a ^ b));
    let pad = padding()?;
    let mut encrypted = vec![0; 8];
    let offered: u32 = if policy == EncryptionPolicy::Require {
        2
    } else {
        3
    };
    encrypted.extend(offered.to_be_bytes());
    encrypted.extend((pad.len() as u16).to_be_bytes());
    encrypted.extend(pad);
    encrypted.extend((initial.len() as u16).to_be_bytes());
    encrypted.extend(initial);
    tx.apply(&mut encrypted);
    request.extend(encrypted);
    send(&mut write, &request).await?;
    let mut marker = [0; 8];
    rx.apply(&mut marker);
    scan(&mut read, &marker).await?;
    let fields = encrypted_read(&mut read, &mut rx, 6).await?;
    let selected = u32::from_be_bytes(fields[..4].try_into().unwrap());
    if !matches!(selected, 1 | 2) || selected & offered == 0 {
        return Err(Error::Policy);
    }
    let pad_length = usize::from(u16::from_be_bytes(fields[4..6].try_into().unwrap()));
    if pad_length > MAX_PAD {
        return Err(Error::Padding);
    }
    encrypted_read(&mut read, &mut rx, pad_length).await?;
    Ok(negotiated(
        read,
        write,
        Some(key),
        if selected == 2 { Some(rx) } else { None },
        if selected == 2 { Some(tx) } else { None },
        Vec::new(),
    ))
}

/// Accept after the caller has read the first 20 bytes and applied its handshake
/// timeout/concurrency limit. The lookup receives SHA1("req2" || SKEY).
pub async fn incoming_after_prefix<
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    F: FnOnce([u8; 20]) -> Option<[u8; 20]>,
>(
    mut read: R,
    mut write: W,
    first: [u8; 20],
    lookup: F,
    policy: EncryptionPolicy,
) -> Result<Negotiated<R, W>, Error> {
    if first == *PLAIN_PREFIX {
        if policy == EncryptionPolicy::Require {
            return Err(Error::Policy);
        }
        return Ok(negotiated(read, write, None, None, None, first.to_vec()));
    }
    if policy == EncryptionPolicy::Disabled {
        return Err(Error::Policy);
    }
    let mut remote = [0; 96];
    remote[..20].copy_from_slice(&first);
    read.read_exact(&mut remote[20..]).await?;
    let exchange = Exchange::new()?;
    let shared = exchange.shared(&remote)?;
    let mut hello = exchange.public().to_vec();
    hello.extend(padding()?);
    send(&mut write, &hello).await?;
    scan(&mut read, &hash(&[b"req1", shared.as_ref()])).await?;
    let mut identifier = [0; 20];
    read.read_exact(&mut identifier).await?;
    let mask = hash(&[b"req3", shared.as_ref()]);
    for (byte, mask) in identifier.iter_mut().zip(mask) {
        *byte ^= mask;
    }
    let key = lookup(identifier).ok_or(Error::UnknownKey)?;
    let mut rx = Cipher::new(b"keyA", shared.as_ref(), &key);
    let mut tx = Cipher::new(b"keyB", shared.as_ref(), &key);
    let header = encrypted_read(&mut read, &mut rx, 14).await?;
    if header[..8] != [0; 8] {
        return Err(Error::Verification);
    }
    let offered = u32::from_be_bytes(header[8..12].try_into().unwrap());
    let selected: u32 = if offered & 2 != 0 {
        2
    } else if offered & 1 != 0 && policy != EncryptionPolicy::Require {
        1
    } else {
        return Err(Error::Policy);
    };
    let pad_length = usize::from(u16::from_be_bytes(header[12..14].try_into().unwrap()));
    if pad_length > MAX_PAD {
        return Err(Error::Padding);
    }
    encrypted_read(&mut read, &mut rx, pad_length).await?;
    let length = encrypted_read(&mut read, &mut rx, 2).await?;
    let initial_length = usize::from(u16::from_be_bytes(length.try_into().unwrap()));
    let initial = encrypted_read(&mut read, &mut rx, initial_length).await?;
    let pad = padding()?;
    let mut response = vec![0; 8];
    response.extend(selected.to_be_bytes());
    response.extend((pad.len() as u16).to_be_bytes());
    response.extend(pad);
    tx.apply(&mut response);
    send(&mut write, &response).await?;
    Ok(negotiated(
        read,
        write,
        Some(key),
        if selected == 2 { Some(rx) } else { None },
        if selected == 2 { Some(tx) } else { None },
        initial,
    ))
}

pub struct EncryptedRead<R> {
    inner: R,
    cipher: Option<Cipher>,
    prefix: Vec<u8>,
    position: usize,
}
impl<R: AsyncRead + Unpin> AsyncRead for EncryptedRead<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.position < this.prefix.len() {
            let count = buf.remaining().min(this.prefix.len() - this.position);
            buf.put_slice(&this.prefix[this.position..this.position + count]);
            this.position += count;
            return Poll::Ready(Ok(()));
        }
        let before = buf.filled().len();
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                if let Some(cipher) = &mut this.cipher {
                    cipher.apply(&mut buf.filled_mut()[before..]);
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

/// Bounded buffered writer. Callers must flush after a complete protocol frame.
pub struct EncryptedWrite<W> {
    inner: W,
    cipher: Option<Cipher>,
    pending: Vec<u8>,
    position: usize,
}
impl<W: AsyncWrite + Unpin> EncryptedWrite<W> {
    fn drain(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.position < self.pending.len() {
            match std::task::ready!(
                Pin::new(&mut self.inner).poll_write(cx, &self.pending[self.position..])
            ) {
                Ok(0) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                Ok(count) => self.position += count,
                Err(error) => return Poll::Ready(Err(error)),
            }
        }
        self.pending.clear();
        self.position = 0;
        Poll::Ready(Ok(()))
    }
}
impl<W: AsyncWrite + Unpin> AsyncWrite for EncryptedWrite<W> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        std::task::ready!(this.drain(cx))?;
        if this.cipher.is_none() {
            return Pin::new(&mut this.inner).poll_write(cx, buf);
        }
        let count = buf.len().min(16384);
        this.pending.extend_from_slice(&buf[..count]);
        this.cipher.as_mut().unwrap().apply(&mut this.pending);
        // The accepted ciphertext remains owned here across partial writes.
        if let Poll::Ready(Err(error)) = this.drain(cx) {
            return Poll::Ready(Err(error));
        }
        Poll::Ready(Ok(count))
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        std::task::ready!(this.drain(cx))?;
        Pin::new(&mut this.inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        std::task::ready!(this.drain(cx))?;
        std::task::ready!(Pin::new(&mut this.inner).poll_flush(cx))?;
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}
