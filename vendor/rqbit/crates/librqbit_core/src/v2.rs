//! BEP 52 metainfo, Merkle trees and authenticated piece verification.
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub const BLOCK_LENGTH: u64 = 16 * 1024;
pub const MAX_METAINFO_LENGTH: usize = 32 * 1024 * 1024;
type Hash = [u8; 32];
type Result<T> = std::result::Result<T, V2Error>;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum V2Error {
    #[error("Invalid torrent metadata: {0}")]
    Invalid(&'static str),
    #[error("Torrent metadata version {0} is not supported.")]
    Version(u64),
    #[error("The piece data does not match its SHA-256 hash.")]
    HashMismatch,
    #[error("Could not read piece data: {0}")]
    Read(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metainfo {
    info_hash: Hash,
    name: String,
    piece_length: u64,
    hybrid: bool,
    total_bytes: u64,
    piece_count: u64,
    files: Vec<File>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    path: Vec<String>,
    length: u64,
    pieces_root: Option<Hash>,
    first_piece: u64,
    piece_count: u64,
    #[serde(skip)]
    piece_hashes: Arc<[Hash]>,
}

impl File {
    pub fn path(&self) -> &[String] {
        &self.path
    }
    pub fn length(&self) -> u64 {
        self.length
    }
    pub fn pieces_root(&self) -> Option<Hash> {
        self.pieces_root
    }
    pub fn first_piece(&self) -> u64 {
        self.first_piece
    }
    pub fn piece_count(&self) -> u64 {
        self.piece_count
    }
}

impl Metainfo {
    /// Returns None for v1. Version detection uses dictionary keys, never byte scanning.
    pub fn parse(bytes: &[u8]) -> Result<Option<Self>> {
        Self::parse_inner(bytes, true)
    }
    fn parse_inner(bytes: &[u8], require_layers: bool) -> Result<Option<Self>> {
        if bytes.len() > MAX_METAINFO_LENGTH {
            return Err(V2Error::Invalid("metadata exceeds 32 MB"));
        }
        let mut parser = Parser {
            bytes,
            at: 0,
            nodes: 0,
            canonical: true,
        };
        let root = parser.value(0)?;
        if parser.at != bytes.len() {
            return Err(V2Error::Invalid("trailing data"));
        }
        let info = root.required(b"info")?;
        let Some(version) = info.get(b"meta version")? else {
            return Ok(None);
        };
        let version = version.number()?;
        if version != 2 {
            return Err(V2Error::Version(version));
        }
        if !parser.canonical {
            return Err(V2Error::Invalid("v2 dictionary keys are not sorted"));
        }
        let piece_length = info.required(b"piece length")?.number()?;
        if piece_length < BLOCK_LENGTH
            || !piece_length.is_power_of_two()
            || piece_length > u32::MAX as u64
        {
            return Err(V2Error::Invalid(
                "piece length must be a power of two from 16 KiB to 2 GiB",
            ));
        }
        let tree = info.required(b"file tree")?;
        let mut files = Vec::new();
        let mut path_budget = MAX_METAINFO_LENGTH;
        walk_tree(tree, &mut Vec::new(), &mut files, &mut path_budget)?;
        if files.is_empty() {
            return Err(V2Error::Invalid("file tree contains no files"));
        }
        let layers: BTreeMap<_, _> = root
            .required(b"piece layers")?
            .dict()?
            .iter()
            .map(|(key, value)| (*key, value))
            .collect();
        let mut used_layers = BTreeMap::<Hash, (u64, Arc<[Hash]>)>::new();
        let mut total_bytes = 0u64;
        let mut piece_count = 0u64;
        for file in &mut files {
            file.first_piece = piece_count;
            file.piece_count = file.length.div_ceil(piece_length);
            total_bytes = total_bytes
                .checked_add(file.length)
                .ok_or(V2Error::Invalid("total length overflows"))?;
            piece_count = piece_count
                .checked_add(file.piece_count)
                .ok_or(V2Error::Invalid("piece count overflows"))?;
            if file.length > piece_length {
                if !require_layers {
                    continue;
                }
                let hash = file.pieces_root.as_ref().unwrap();
                let value = layers
                    .get(hash.as_slice())
                    .ok_or(V2Error::Invalid("missing piece layer"))?
                    .bytes()?;
                if value.len() as u64 / 32 != file.piece_count || value.len() % 32 != 0 {
                    return Err(V2Error::Invalid(
                        "piece layer has the wrong number of hashes",
                    ));
                }
                if let Some((length, hashes)) = used_layers.get(hash) {
                    if *length != file.length {
                        return Err(V2Error::Invalid(
                            "shared pieces root has inconsistent file lengths",
                        ));
                    }
                    file.piece_hashes = hashes.clone();
                } else {
                    let hashes: Vec<Hash> = value.as_chunks::<32>().0.to_vec();
                    let padding = zero_hash(piece_length / BLOCK_LENGTH);
                    if merkle_root(hashes.clone(), padding) != *hash {
                        return Err(V2Error::HashMismatch);
                    }
                    file.piece_hashes = hashes.into();
                    used_layers.insert(*hash, (file.length, file.piece_hashes.clone()));
                }
            }
        }
        if require_layers && layers.len() != used_layers.len() {
            return Err(V2Error::Invalid("unreferenced piece layer"));
        }
        let hybrid = info.get(b"pieces")?.is_some();
        if hybrid {
            validate_hybrid(info, &files, piece_length)?;
        } else if info.get(b"files")?.is_some() || info.get(b"length")?.is_some() {
            return Err(V2Error::Invalid("v1 file layout without v1 piece hashes"));
        }
        let name = info
            .get(b"name")?
            .map(Value::bytes)
            .transpose()?
            .map(|v| String::from_utf8_lossy(v).into_owned())
            .unwrap_or_else(|| "Untitled torrent".into());
        Ok(Some(Self {
            info_hash: Sha256::digest(info.raw).into(),
            name,
            piece_length,
            hybrid,
            total_bytes,
            piece_count,
            files,
        }))
    }

    pub fn piece_layers(&self) -> BTreeMap<Hash, Vec<u8>> {
        self.files
            .iter()
            .filter(|f| !f.piece_hashes.is_empty())
            .map(|f| {
                (
                    f.pieces_root.unwrap(),
                    f.piece_hashes.iter().flatten().copied().collect(),
                )
            })
            .collect()
    }
    pub fn info_hash(&self) -> Hash {
        self.info_hash
    }
    pub fn is_hybrid(&self) -> bool {
        self.hybrid
    }
    pub fn files(&self) -> &[File] {
        &self.files
    }
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }
    pub fn piece_count(&self) -> u64 {
        self.piece_count
    }
    pub fn piece_length(&self) -> u64 {
        self.piece_length
    }

    fn request_file(&self, request: &HashRequest) -> Result<(usize, &File, u32)> {
        if !request.valid() {
            return Err(V2Error::Invalid("invalid hash request"));
        }
        let (index, file) = self
            .files
            .iter()
            .enumerate()
            .find(|(_, f)| f.pieces_root == Some(request.pieces_root))
            .ok_or(V2Error::Invalid("unknown pieces root"))?;
        let height = file
            .length
            .div_ceil(BLOCK_LENGTH)
            .next_power_of_two()
            .ilog2();
        if request.base_layer >= height
            || request.proof_layers >= height - request.base_layer
            || u64::from(request.index) + u64::from(request.length)
                > (1u64 << (height - request.base_layer))
        {
            return Err(V2Error::Invalid("hash request is outside the file tree"));
        }
        Ok((index, file, height))
    }

    /// Produce base nodes followed by the uncle hashes defined by BEP 52.
    pub fn serve_hashes(
        &self,
        request: &HashRequest,
        mut read: impl FnMut(usize, u64, &mut [u8]) -> std::io::Result<()>,
    ) -> Result<Vec<Hash>> {
        let (file_index, file, _) = self.request_file(request)?;
        let piece_layer = (self.piece_length / BLOCK_LENGTH).ilog2();
        fn node(
            file: &File,
            file_index: usize,
            layer: u32,
            index: u64,
            piece_layer: u32,
            read: &mut impl FnMut(usize, u64, &mut [u8]) -> std::io::Result<()>,
        ) -> Result<Hash> {
            if layer == piece_layer && !file.piece_hashes.is_empty() {
                return Ok(file
                    .piece_hashes
                    .get(index as usize)
                    .copied()
                    .unwrap_or_else(|| zero_hash(1u64 << layer)));
            }
            let start = index * (BLOCK_LENGTH << layer);
            if start >= file.length {
                return Ok(zero_hash(1u64 << layer));
            }
            if layer == 0 {
                let mut block = [0u8; BLOCK_LENGTH as usize];
                let length = (file.length - start).min(BLOCK_LENGTH) as usize;
                read(file_index, start, &mut block[..length])
                    .map_err(|e| V2Error::Read(e.to_string()))?;
                return Ok(Sha256::digest(&block[..length]).into());
            }
            Ok(pair(
                node(file, file_index, layer - 1, index * 2, piece_layer, read)?,
                node(
                    file,
                    file_index,
                    layer - 1,
                    index * 2 + 1,
                    piece_layer,
                    read,
                )?,
            ))
        }
        let mut hashes = Vec::with_capacity(request.length as usize + 32);
        for index in request.index..request.index + request.length {
            hashes.push(node(
                file,
                file_index,
                request.base_layer,
                index as u64,
                piece_layer,
                &mut read,
            )?);
        }
        let mut layer = request.base_layer + request.length.ilog2();
        let mut index = request.index >> request.length.ilog2();
        let target = request.base_layer + request.proof_layers + 1;
        while layer < target {
            hashes.push(node(
                file,
                file_index,
                layer,
                (index ^ 1) as u64,
                piece_layer,
                &mut read,
            )?);
            index >>= 1;
            layer += 1;
        }
        Ok(hashes)
    }

    /// Authenticate a correlated, complete proof against the root from the info dictionary.
    pub fn verify_hashes(&self, request: &HashRequest, hashes: &[Hash]) -> Result<()> {
        let (_, _, height) = self.request_file(request)?;
        let subtree_height = request.length.ilog2();
        let proof_count = (request.proof_layers + 1).saturating_sub(subtree_height) as usize;
        if request.base_layer + request.proof_layers + 1 != height
            || hashes.len() != request.length as usize + proof_count
        {
            return Err(V2Error::Invalid(
                "incomplete or incorrectly sized root proof",
            ));
        }
        let mut computed = merkle_root(hashes[..request.length as usize].to_vec(), [0; 32]);
        let mut index = request.index >> subtree_height;
        for uncle in &hashes[request.length as usize..] {
            computed = if index & 1 == 0 {
                pair(computed, *uncle)
            } else {
                pair(*uncle, computed)
            };
            index >>= 1;
        }
        if computed != request.pieces_root {
            return Err(V2Error::HashMismatch);
        }
        Ok(())
    }

    pub fn piece_size(&self, index: u64) -> Result<u64> {
        let file = self
            .files
            .iter()
            .find(|f| index >= f.first_piece && index - f.first_piece < f.piece_count)
            .ok_or(V2Error::Invalid("piece index out of range"))?;
        Ok((file.length - (index - file.first_piece) * self.piece_length).min(self.piece_length))
    }

    /// Read and verify a complete wire piece using a bounded block buffer.
    pub fn verify_piece_reader(
        &self,
        index: u64,
        mut read: impl FnMut(u64, &mut [u8]) -> std::io::Result<()>,
    ) -> Result<()> {
        let file = self
            .files
            .iter()
            .find(|f| index >= f.first_piece && index - f.first_piece < f.piece_count)
            .ok_or(V2Error::Invalid("piece index out of range"))?;
        let local = index - file.first_piece;
        let length = self.piece_size(index)?;
        let mut block = [0u8; BLOCK_LENGTH as usize];
        let mut hashes = Vec::new();
        let mut offset = 0;
        while offset < length {
            let count = (length - offset).min(BLOCK_LENGTH) as usize;
            read(offset, &mut block[..count]).map_err(|e| V2Error::Read(e.to_string()))?;
            hashes.push(Sha256::digest(&block[..count]).into());
            offset += count as u64;
        }
        let expected = if file.length <= self.piece_length {
            file.pieces_root.unwrap()
        } else {
            hashes.resize((self.piece_length / BLOCK_LENGTH) as usize, [0; 32]);
            *file
                .piece_hashes
                .get(local as usize)
                .ok_or(V2Error::Invalid("piece layer unavailable"))?
        };
        if merkle_root(hashes, [0; 32]) != expected {
            return Err(V2Error::HashMismatch);
        }
        Ok(())
    }

    pub fn verify_piece(&self, index: u64, data: &[u8]) -> Result<()> {
        if data.len() as u64 != self.piece_size(index)? {
            return Err(V2Error::Invalid("incorrect piece data length"));
        }
        self.verify_piece_reader(index, |offset, output| {
            output.copy_from_slice(&data[offset as usize..offset as usize + output.len()]);
            Ok(())
        })
    }
}

fn pair(left: Hash, right: Hash) -> Hash {
    let mut hash = Sha256::new();
    hash.update(left);
    hash.update(right);
    hash.finalize().into()
}

fn zero_hash(mut blocks: u64) -> Hash {
    let mut hash = [0; 32];
    while blocks > 1 {
        hash = pair(hash, hash);
        blocks /= 2;
    }
    hash
}

fn merkle_root(mut hashes: Vec<Hash>, padding: Hash) -> Hash {
    debug_assert!(!hashes.is_empty());
    hashes.resize(hashes.len().next_power_of_two(), padding);
    while hashes.len() > 1 {
        for index in 0..hashes.len() / 2 {
            hashes[index] = pair(hashes[index * 2], hashes[index * 2 + 1]);
        }
        hashes.truncate(hashes.len() / 2);
    }
    hashes[0]
}

fn component(bytes: &[u8]) -> Result<String> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_| V2Error::Invalid("file names must be valid UTF-8 on this platform"))?;
    if value.len() > 255
        || value.is_empty()
        || matches!(value, "." | "..")
        || value.bytes().any(|b| matches!(b, 0 | b'/' | b'\\'))
    {
        return Err(V2Error::Invalid("unsafe file path component"));
    }
    Ok(value.into())
}

fn walk_tree(
    tree: &Value<'_>,
    path: &mut Vec<String>,
    files: &mut Vec<File>,
    path_budget: &mut usize,
) -> Result<()> {
    let entries = tree.dict()?;
    if let Some((_, properties)) = entries.iter().find(|(key, _)| key.is_empty()) {
        if path.is_empty() || entries.len() != 1 {
            return Err(V2Error::Invalid(
                "file tree node is both a file and a directory",
            ));
        }
        // Symlinks need separate storage semantics and are deliberately not followed.
        if properties.get(b"symlink path")?.is_some()
            || properties
                .get(b"attr")?
                .map(Value::bytes)
                .transpose()?
                .is_some_and(|a| a.contains(&b'l'))
        {
            return Err(V2Error::Invalid("symbolic links are not supported"));
        }
        let length = properties.required(b"length")?.number()?;
        let pieces_root = properties
            .get(b"pieces root")?
            .map(|v| {
                v.bytes()?
                    .try_into()
                    .map_err(|_| V2Error::Invalid("pieces root must be 32 bytes"))
            })
            .transpose()?;
        if (length == 0) != pieces_root.is_none() {
            return Err(V2Error::Invalid(
                "nonempty files require a root; empty files must omit it",
            ));
        }
        // A long shared prefix must not expand a small input into unbounded owned paths.
        let path_length = path.iter().map(|part| part.len() + 1).sum::<usize>();
        *path_budget = path_budget
            .checked_sub(path_length)
            .ok_or(V2Error::Invalid("total file paths exceed 32 MiB"))?;
        files.push(File {
            path: path.clone(),
            length,
            pieces_root,
            first_piece: 0,
            piece_count: 0,
            piece_hashes: Arc::from([]),
        });
    } else {
        if entries.is_empty() {
            return Err(V2Error::Invalid("empty directory in file tree"));
        }
        for (key, child) in entries {
            path.push(component(key)?);
            if path.iter().map(|part| part.len() + 1).sum::<usize>() > 4096 {
                return Err(V2Error::Invalid("file path exceeds 4096 bytes"));
            }
            walk_tree(child, path, files, path_budget)?;
            path.pop();
        }
    }
    Ok(())
}

fn validate_hybrid(info: &Value<'_>, files: &[File], piece_length: u64) -> Result<()> {
    let pieces = info.required(b"pieces")?.bytes()?;
    let mut offset = 0u64;
    if let Some(length) = info.get(b"length")? {
        if info.get(b"symlink path")?.is_some()
            || info
                .get(b"attr")?
                .map(Value::bytes)
                .transpose()?
                .is_some_and(|a| a.contains(&b'l'))
        {
            return Err(V2Error::Invalid("symbolic links are not supported"));
        }
        if info.get(b"files")?.is_some()
            || files.len() != 1
            || files[0].path.len() != 1
            || length.number()? != files[0].length
            || info.required(b"name")?.bytes()? != files[0].path[0].as_bytes()
        {
            return Err(V2Error::Invalid(
                "hybrid single-file layout differs from v2",
            ));
        }
        offset = length.number()?;
    } else {
        let mut next_file = 0;
        let mut previous_was_padding = false;
        let mut paths = BTreeSet::new();
        for entry in info.required(b"files")?.list()? {
            let length = entry.required(b"length")?.number()?;
            let path: Vec<String> = entry
                .required(b"path")?
                .list()?
                .iter()
                .map(|v| component(v.bytes()?))
                .collect::<Result<_>>()?;
            if path.is_empty() || !paths.insert(path.clone()) {
                return Err(V2Error::Invalid("duplicate or empty hybrid path"));
            }
            let attr = entry
                .get(b"attr")?
                .map(Value::bytes)
                .transpose()?
                .unwrap_or_default();
            if attr.contains(&b'l') || entry.get(b"symlink path")?.is_some() {
                return Err(V2Error::Invalid("symbolic links are not supported"));
            }
            if attr.contains(&b'p') {
                let padding = (piece_length - offset % piece_length) % piece_length;
                if next_file == 0 || previous_was_padding || padding == 0 || length != padding {
                    return Err(V2Error::Invalid("incorrect hybrid padding"));
                }
                previous_was_padding = true;
            } else {
                let file = files
                    .get(next_file)
                    .ok_or(V2Error::Invalid("extra v1 file in hybrid"))?;
                if file.path != path
                    || file.length != length
                    || (length != 0 && !offset.is_multiple_of(piece_length))
                {
                    return Err(V2Error::Invalid(
                        "hybrid file order, length or alignment differs from v2",
                    ));
                }
                next_file += 1;
                previous_was_padding = false;
            }
            offset = offset
                .checked_add(length)
                .ok_or(V2Error::Invalid("hybrid size overflows"))?;
        }
        if next_file != files.len() {
            return Err(V2Error::Invalid("missing v1 file in hybrid"));
        }
        // A padding filename must not alias a real file or one of its parent directories.
        for path in &paths {
            for end in 1..path.len() {
                if paths.contains(&path[..end]) {
                    return Err(V2Error::Invalid("hybrid file/directory path collision"));
                }
            }
        }
    }
    if pieces.len() % 20 != 0 || pieces.len() as u64 / 20 != offset.div_ceil(piece_length) {
        return Err(V2Error::Invalid("incorrect v1 piece hash count in hybrid"));
    }
    Ok(())
}

// Borrow raw spans for hashing, retain dictionary order and reject duplicates.
// Bound recursion and node count before allocation, including ignored extension fields.
struct Value<'a> {
    raw: &'a [u8],
    kind: Kind<'a>,
}
enum Kind<'a> {
    Bytes(&'a [u8]),
    Number(&'a [u8]),
    List(Vec<Value<'a>>),
    Dict(Vec<(&'a [u8], Value<'a>)>),
}
impl<'a> Value<'a> {
    fn dict(&self) -> Result<&[(&'a [u8], Value<'a>)]> {
        match &self.kind {
            Kind::Dict(v) => Ok(v),
            _ => Err(V2Error::Invalid("expected dictionary")),
        }
    }
    fn list(&self) -> Result<&[Value<'a>]> {
        match &self.kind {
            Kind::List(v) => Ok(v),
            _ => Err(V2Error::Invalid("expected list")),
        }
    }
    fn bytes(&self) -> Result<&'a [u8]> {
        match self.kind {
            Kind::Bytes(v) => Ok(v),
            _ => Err(V2Error::Invalid("expected byte string")),
        }
    }
    fn number(&self) -> Result<u64> {
        match self.kind {
            Kind::Number(v) => std::str::from_utf8(v)
                .unwrap()
                .parse()
                .map_err(|_| V2Error::Invalid("expected unsigned 64-bit integer")),
            _ => Err(V2Error::Invalid("expected integer")),
        }
    }
    fn get(&self, key: &[u8]) -> Result<Option<&Value<'a>>> {
        Ok(self.dict()?.iter().find(|(k, _)| *k == key).map(|(_, v)| v))
    }
    fn required(&self, key: &[u8]) -> Result<&Value<'a>> {
        self.get(key)?
            .ok_or(V2Error::Invalid("missing required field"))
    }
}
struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
    nodes: usize,
    canonical: bool,
}
impl<'a> Parser<'a> {
    fn value(&mut self, depth: usize) -> Result<Value<'a>> {
        self.nodes += 1;
        if depth > 64 || self.nodes > 200_000 {
            return Err(V2Error::Invalid("metadata nesting or entry limit exceeded"));
        }
        let start = self.at;
        let tag = *self
            .bytes
            .get(self.at)
            .ok_or(V2Error::Invalid("truncated bencode"))?;
        self.at += 1;
        let kind = match tag {
            b'd' => {
                let mut entries = Vec::new();
                let mut seen = BTreeSet::new();
                while self.bytes.get(self.at) != Some(&b'e') {
                    let key = self.value(depth + 1)?.bytes()?;
                    if !seen.insert(key) {
                        return Err(V2Error::Invalid("duplicate dictionary key"));
                    }
                    if entries.last().is_some_and(|(previous, _)| *previous > key) {
                        self.canonical = false;
                    }
                    entries.push((key, self.value(depth + 1)?));
                }
                self.at += 1;
                Kind::Dict(entries)
            }
            b'l' => {
                let mut values = Vec::new();
                while self.bytes.get(self.at) != Some(&b'e') {
                    values.push(self.value(depth + 1)?);
                }
                self.at += 1;
                Kind::List(values)
            }
            b'i' => {
                let end = self.bytes[self.at..]
                    .iter()
                    .position(|b| *b == b'e')
                    .ok_or(V2Error::Invalid("unterminated integer"))?
                    + self.at;
                let number = &self.bytes[self.at..end];
                let digits = number.strip_prefix(b"-").unwrap_or(number);
                if digits.is_empty()
                    || !digits.iter().all(u8::is_ascii_digit)
                    || (digits.len() > 1 && digits[0] == b'0')
                    || number == b"-0"
                {
                    return Err(V2Error::Invalid("noncanonical integer"));
                }
                self.at = end + 1;
                Kind::Number(number)
            }
            b'0'..=b'9' => {
                let end = self.bytes[start..]
                    .iter()
                    .position(|b| *b == b':')
                    .ok_or(V2Error::Invalid("missing string length"))?
                    + start;
                let digits = &self.bytes[start..end];
                if !digits.iter().all(u8::is_ascii_digit) || (digits.len() > 1 && digits[0] == b'0')
                {
                    return Err(V2Error::Invalid("noncanonical string length"));
                }
                let length: usize = std::str::from_utf8(digits)
                    .unwrap()
                    .parse()
                    .map_err(|_| V2Error::Invalid("string length overflows"))?;
                self.at = (end + 1)
                    .checked_add(length)
                    .filter(|n| *n <= self.bytes.len())
                    .ok_or(V2Error::Invalid("truncated string"))?;
                Kind::Bytes(&self.bytes[end + 1..self.at])
            }
            _ => return Err(V2Error::Invalid("unknown bencode tag")),
        };
        Ok(Value {
            raw: &self.bytes[start..self.at],
            kind,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HashRequest {
    pub pieces_root: [u8; 32],
    pub base_layer: u32,
    pub index: u32,
    pub length: u32,
    pub proof_layers: u32,
}
impl HashRequest {
    pub fn valid(&self) -> bool {
        self.base_layer < 32
            && self.proof_layers < 32
            && self.base_layer + self.proof_layers < 32
            && self.length >= 2
            && self.length <= 512
            && self.length.is_power_of_two()
            && self.index.is_multiple_of(self.length)
            && self.index.checked_add(self.length).is_some()
    }
    pub fn encode(&self, buf: &mut [u8]) {
        buf[..32].copy_from_slice(&self.pieces_root);
        for (chunk, value) in buf[32..48].as_chunks_mut::<4>().0.iter_mut().zip([
            self.base_layer,
            self.index,
            self.length,
            self.proof_layers,
        ]) {
            *chunk = value.to_be_bytes();
        }
    }
    pub fn decode(buf: [u8; 48]) -> Self {
        Self {
            pieces_root: buf[..32].try_into().unwrap(),
            base_layer: u32::from_be_bytes(buf[32..36].try_into().unwrap()),
            index: u32::from_be_bytes(buf[36..40].try_into().unwrap()),
            length: u32::from_be_bytes(buf[40..44].try_into().unwrap()),
            proof_layers: u32::from_be_bytes(buf[44..48].try_into().unwrap()),
        }
    }
}

/// Metadata from ut_metadata has authenticated roots but no external piece layers.
/// This type cannot be used for payload verification until every requested proof passes.
pub struct PendingMetainfo {
    meta: Metainfo,
    requests: std::collections::VecDeque<HashRequest>,
    layers: BTreeMap<Hash, Vec<Hash>>,
}
impl PendingMetainfo {
    pub fn from_info(info: &[u8]) -> Result<Option<Self>> {
        let bytes = [b"d4:info".as_slice(), info, b"12:piece layersdee"].concat();
        let Some(meta) = Metainfo::parse_inner(&bytes, false)? else {
            return Ok(None);
        };
        let mut layers = BTreeMap::new();
        let mut requests = std::collections::VecDeque::new();
        let mut total = 0u64;
        for file in &meta.files {
            if file.length <= meta.piece_length {
                continue;
            }
            let root = file.pieces_root.unwrap();
            if layers.contains_key(&root) {
                continue;
            }
            total = total
                .checked_add(file.piece_count * 32)
                .ok_or(V2Error::Invalid("piece layers overflow"))?;
            if total > MAX_METAINFO_LENGTH as u64 {
                return Err(V2Error::Invalid("piece layers exceed 32 MiB"));
            }
            layers.insert(root, vec![[0; 32]; file.piece_count as usize]);
            let base = (meta.piece_length / BLOCK_LENGTH).ilog2();
            let height = file
                .length
                .div_ceil(BLOCK_LENGTH)
                .next_power_of_two()
                .ilog2();
            let mut index = 0u64;
            while index < file.piece_count {
                let length = (file.piece_count - index)
                    .min(512)
                    .next_power_of_two()
                    .max(2);
                let request = HashRequest {
                    pieces_root: root,
                    base_layer: base,
                    index: index
                        .try_into()
                        .map_err(|_| V2Error::Invalid("hash index overflows"))?,
                    length: length as u32,
                    proof_layers: height - base - 1,
                };
                meta.request_file(&request)?;
                requests.push_back(request);
                index += length;
            }
        }
        Ok(Some(Self {
            meta,
            requests,
            layers,
        }))
    }
    pub fn info_hash(&self) -> Hash {
        self.meta.info_hash
    }
    pub fn next_request(&self) -> Option<HashRequest> {
        self.requests.front().copied()
    }
    pub fn accept(&mut self, request: HashRequest, hashes: &[Hash]) -> Result<()> {
        if self.next_request() != Some(request) {
            return Err(V2Error::Invalid("unsolicited hash response"));
        }
        self.meta.verify_hashes(&request, hashes)?;
        let layer = self.layers.get_mut(&request.pieces_root).unwrap();
        let start = request.index as usize;
        let count = (layer.len() - start).min(request.length as usize);
        layer[start..start + count].copy_from_slice(&hashes[..count]);
        self.requests.pop_front();
        Ok(())
    }
    pub fn finish(mut self) -> Result<Metainfo> {
        if !self.requests.is_empty() {
            return Err(V2Error::Invalid("piece layers are incomplete"));
        }
        let mut shared = BTreeMap::<Hash, Arc<[Hash]>>::new();
        for file in &mut self.meta.files {
            if file.length > self.meta.piece_length {
                let root = file.pieces_root.unwrap();
                let layer = shared
                    .entry(root)
                    .or_insert_with(|| self.layers.remove(&root).unwrap().into());
                file.piece_hashes = layer.clone();
            }
        }
        Ok(self.meta)
    }
}
pub fn sha256(bytes: &[u8]) -> Hash {
    Sha256::digest(bytes).into()
}
