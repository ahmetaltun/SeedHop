pub mod json;
#[cfg(feature = "postgres")]
pub mod postgres;

use std::{collections::HashSet, path::PathBuf};

use anyhow::Context;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use librqbit_core::Id20;
use librqbit_core::magnet::Magnet;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    AddTorrent, AddTorrentOptions, bitv_factory::BitVFactory, session::TorrentId,
    torrent_state::ManagedTorrentHandle,
};

#[derive(Serialize, Deserialize, Clone)]
pub struct SerializedTorrent {
    #[serde(
        serialize_with = "serialize_info_hash",
        deserialize_with = "deserialize_info_hash"
    )]
    info_hash: Id20,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    info_hash_v2: Option<String>,
    #[serde(skip)]
    torrent_bytes: Bytes,
    trackers: HashSet<String>,
    output_folder: PathBuf,
    only_files: Option<Vec<usize>>,
    is_paused: bool,
    #[serde(default)]
    is_queued: bool,
    #[serde(default)]
    file_names: std::collections::HashMap<usize, PathBuf>,
    #[serde(default)]
    file_name_mapping_version: u8,
    #[serde(default)]
    file_priorities: Vec<u8>,
    #[serde(default)]
    sequential: bool,
    #[serde(default)]
    first_last_pieces: bool,
}

impl SerializedTorrent {
    pub fn info_hash(&self) -> &Id20 {
        &self.info_hash
    }
    pub fn into_add_torrent(mut self) -> anyhow::Result<(AddTorrent<'static>, AddTorrentOptions)> {
        if let Some(expected) = &self.info_hash_v2 {
            anyhow::ensure!(
                !self.torrent_bytes.is_empty(),
                "v2 metadata file is missing; restore the preserved .torrent file"
            );
            let parsed = librqbit_core::torrent_metainfo::torrent_from_bytes(&self.torrent_bytes)?;
            let v2 = parsed
                .info
                .data
                .v2
                .as_ref()
                .context("saved v2 torrent contains v1 metadata")?;
            anyhow::ensure!(
                librqbit_core::Id32::new(v2.info_hash()).as_string() == *expected
                    && parsed.info_hash == self.info_hash,
                "saved v2 identity does not match metadata"
            );
            anyhow::ensure!(
                self.file_name_mapping_version <= 1,
                "saved filename mapping version is unsupported"
            );
            if self.file_name_mapping_version == 0 {
                let paths = librqbit_core::v2::Metainfo::legacy_storage_paths(&self.torrent_bytes)?
                    .context("saved v2 metadata has no legacy layout")?;
                for (index, path) in paths.into_iter().enumerate() {
                    self.file_names
                        .entry(index)
                        .or_insert_with(|| path.into_iter().collect());
                }
            } else {
                anyhow::ensure!(
                    self.file_names.len() == v2.files().len()
                        && (0..v2.files().len()).all(|i| self.file_names.contains_key(&i)),
                    "saved filename map is incomplete"
                );
            }
        }
        let add_torrent = if !self.torrent_bytes.is_empty() {
            AddTorrent::TorrentFileBytes(self.torrent_bytes)
        } else {
            let magnet = Magnet::from_id20(
                self.info_hash,
                self.trackers.into_iter().collect(),
                self.only_files.clone(),
            )
            .to_string();
            AddTorrent::from_url(magnet)
        };

        let opts = AddTorrentOptions {
            paused: self.is_paused,
            queued: self.is_queued,
            file_names: self.file_names,
            file_priorities: self.file_priorities,
            sequential: self.sequential,
            first_last_pieces: self.first_last_pieces,
            output_folder: Some(
                self.output_folder
                    .to_str()
                    .context("broken path")?
                    .to_owned(),
            ),
            only_files: self.only_files,
            overwrite: true,
            ..Default::default()
        };

        Ok((add_torrent, opts))
    }
}

// TODO: make this info_hash first, ID-second.
#[async_trait]
pub trait SessionPersistenceStore: core::fmt::Debug + Send + Sync + BitVFactory {
    async fn next_id(&self) -> anyhow::Result<TorrentId>;
    async fn store(&self, id: TorrentId, torrent: &ManagedTorrentHandle) -> anyhow::Result<()>;
    async fn delete(&self, id: TorrentId) -> anyhow::Result<()>;
    async fn get(&self, id: TorrentId) -> anyhow::Result<SerializedTorrent>;
    async fn update_metadata(
        &self,
        id: TorrentId,
        torrent: &ManagedTorrentHandle,
    ) -> anyhow::Result<()>;
    async fn stream_all(
        &self,
    ) -> anyhow::Result<BoxStream<'_, anyhow::Result<(TorrentId, SerializedTorrent)>>>;
}

fn serialize_info_hash<S>(id: &Id20, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    id.as_string().serialize(serializer)
}

fn deserialize_info_hash<'de, D>(deserializer: D) -> Result<Id20, D::Error>
where
    D: Deserializer<'de>,
{
    Id20::deserialize(deserializer)
}

#[cfg(test)]
mod preference_tests {
    use super::*;

    #[test]
    fn saved_preferences_restore_and_legacy_records_default_to_normal() {
        let legacy = serde_json::json!({
            "info_hash": "0000000000000000000000000000000000000000",
            "trackers": [], "output_folder": "/tmp", "only_files": [0], "is_paused": true
        });
        for (sequential, first_last) in [(false, false), (true, false), (false, true), (true, true)]
        {
            let mut value = legacy.clone();
            if sequential {
                value["sequential"] = true.into();
            }
            if first_last {
                value["first_last_pieces"] = true.into();
            }
            let saved: SerializedTorrent = serde_json::from_value(value).unwrap();
            let (_, options) = saved.into_add_torrent().unwrap();
            assert_eq!(options.sequential, sequential);
            assert_eq!(options.first_last_pieces, first_last);
            assert!(options.paused);
            assert_eq!(options.only_files, Some(vec![0]));
        }
    }
}
