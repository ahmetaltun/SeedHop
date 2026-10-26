//! Hold the first verification read after a completed payload write.
use librqbit::{
    ManagedTorrentShared, TorrentMetadata,
    storage::{
        BoxStorageFactory, StorageFactory, StorageFactoryExt, TorrentStorage,
        filesystem::FilesystemStorageFactory,
    },
};
use std::{
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[derive(Default)]
pub struct Gate {
    written: AtomicUsize,
    pub verifying: AtomicBool,
    released: Mutex<bool>,
    wake: Condvar,
}
impl Gate {
    pub fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}

#[derive(Clone)]
pub struct Factory(pub Arc<Gate>);
impl StorageFactory for Factory {
    type Storage = Storage;
    fn create(
        &self,
        shared: &ManagedTorrentShared,
        metadata: &TorrentMetadata,
    ) -> anyhow::Result<Storage> {
        Ok(Storage {
            inner: Box::new(FilesystemStorageFactory::default().create(shared, metadata)?),
            gate: self.0.clone(),
        })
    }
    fn clone_box(&self) -> BoxStorageFactory {
        self.clone().boxed()
    }
}

pub struct Storage {
    inner: Box<dyn TorrentStorage>,
    gate: Arc<Gate>,
}
impl TorrentStorage for Storage {
    fn init(
        &mut self,
        shared: &ManagedTorrentShared,
        metadata: &TorrentMetadata,
    ) -> anyhow::Result<()> {
        self.inner.init(shared, metadata)
    }
    fn pread_exact(&self, file: usize, offset: u64, buf: &mut [u8]) -> anyhow::Result<()> {
        if file == 1
            && self.gate.written.load(Ordering::Acquire) >= 32768
            && !self.gate.verifying.swap(true, Ordering::AcqRel)
        {
            let released = self.gate.released.lock().unwrap();
            let (released, _) = self
                .gate
                .wake
                .wait_timeout_while(released, std::time::Duration::from_secs(5), |released| {
                    !*released
                })
                .unwrap();
            anyhow::ensure!(*released, "verification gate timed out");
        }
        self.inner.pread_exact(file, offset, buf)
    }
    fn pwrite_all(&self, file: usize, offset: u64, buf: &[u8]) -> anyhow::Result<()> {
        self.inner.pwrite_all(file, offset, buf)?;
        if file == 1 {
            self.gate.written.fetch_add(buf.len(), Ordering::Release);
        }
        Ok(())
    }
    fn remove_file(&self, file: usize, path: &Path) -> anyhow::Result<()> {
        self.inner.remove_file(file, path)
    }
    fn remove_directory_if_empty(&self, path: &Path) -> anyhow::Result<()> {
        self.inner.remove_directory_if_empty(path)
    }
    fn ensure_file_length(&self, file: usize, length: u64) -> anyhow::Result<()> {
        self.inner.ensure_file_length(file, length)
    }
    fn take(&self) -> anyhow::Result<Box<dyn TorrentStorage>> {
        Ok(Box::new(Storage {
            inner: self.inner.take()?,
            gate: self.gate.clone(),
        }))
    }
}
