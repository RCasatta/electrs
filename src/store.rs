use rocksdb;

use std::path::{Path, PathBuf};

use hex;
use util::Bytes;

#[derive(Clone)]
pub struct Row {
    pub key: Bytes,
    pub value: Bytes,
}

impl Row {
    pub fn into_pair(self) -> (Bytes, Bytes) {
        (self.key, self.value)
    }
}

pub trait ReadStore: Sync {
    fn get(&self, key: &[u8]) -> Option<Bytes>;
    fn scan(&self, prefix: &[u8]) -> Vec<Row>;
}

pub trait WriteStore: Sync {
    fn write(&self, rows: Vec<Row>);
    fn flush(&self);
}

#[derive(Clone)]
struct Options {
    path: PathBuf,
    bulk_import: bool,
}

pub struct DBStore {
    db: rocksdb::DB,
    opts: Options,
}

impl DBStore {
    fn open_opts(opts: Options) -> Self {
        debug!("opening DB at {:?}", opts.path);
        let mut db_opts = rocksdb::Options::default();
        db_opts.create_if_missing(true);
        // db_opts.set_keep_log_file_num(10);
        db_opts.set_max_open_files(2048);
        db_opts.set_compaction_readahead_size(1 << 20);
        db_opts.set_compaction_style(rocksdb::DBCompactionStyle::Level);
        db_opts.set_compression_type(rocksdb::DBCompressionType::Snappy);
        db_opts.set_target_file_size_base(128 << 20);
        db_opts.set_write_buffer_size(64 << 20);
        db_opts.set_min_write_buffer_number(2);
        db_opts.set_max_write_buffer_number(3);
        db_opts.set_disable_auto_compactions(opts.bulk_import); // for initial bulk load

        let mut block_opts = rocksdb::BlockBasedOptions::default();
        block_opts.set_block_size(1 << 20);
        DBStore {
            db: rocksdb::DB::open(&db_opts, &opts.path).unwrap(),
            opts,
        }
    }

    /// Opens a new RocksDB at the specified location.
    pub fn open(path: &Path) -> Self {
        DBStore::open_opts(Options {
            path: path.to_path_buf(),
            bulk_import: true,
        })
    }

    pub fn enable_compaction(self) -> Self {
        let mut opts = self.opts.clone();
        opts.bulk_import = false;
        drop(self);
        // DB must be closed before being re-opened:
        DBStore::open_opts(opts)
    }

    pub fn put(&self, key: &[u8], value: &[u8]) {
        self.db.put(key, value).unwrap();
    }

    pub fn compact(&self) {
        info!("starting full compaction");
        self.db.compact_range(None, None); // would take a while
        info!("finished full compaction");
    }

    pub fn max_collision(&self, prefix: &[u8]) {
        let prefix_len = prefix.len();
        let mut iter = self.db.raw_iterator();
        iter.seek(prefix);
        let mut prev: Option<Vec<u8>> = None;
        let mut collision_max = 0;
        while iter.valid() {
            let key = &iter.key().unwrap();
            if !key.starts_with(prefix) {
                break;
            }
            if let Some(prev) = prev {
                let collision_len = prev.iter()
                    .zip(key.iter())
                    .take_while(|(a, b)| a == b)
                    .count();
                if collision_len > collision_max {
                    eprintln!(
                        "{} bytes collision found:\n{:?}\n{:?}\n",
                        collision_len - prefix_len,
                        revhex(&prev[prefix_len..]),
                        revhex(&key[prefix_len..]),
                    );
                    collision_max = collision_len;
                }
            }
            prev = Some(key.to_vec());
            iter.next();
        }
    }
}

fn revhex(value: &[u8]) -> String {
    hex::encode(&value.iter().cloned().rev().collect::<Vec<u8>>())
}

impl ReadStore for DBStore {
    fn get(&self, key: &[u8]) -> Option<Bytes> {
        self.db.get(key).unwrap().map(|v| v.to_vec())
    }

    // TODO: use generators
    fn scan(&self, prefix: &[u8]) -> Vec<Row> {
        let mut rows = vec![];
        for (key, value) in self.db.iterator(rocksdb::IteratorMode::From(
            prefix,
            rocksdb::Direction::Forward,
        )) {
            if !key.starts_with(prefix) {
                break;
            }
            rows.push(Row {
                key: key.to_vec(),
                value: value.to_vec(),
            });
        }
        rows
    }
}

impl WriteStore for DBStore {
    fn write(&self, rows: Vec<Row>) {
        let mut batch = rocksdb::WriteBatch::default();
        for row in rows {
            batch.put(row.key.as_slice(), row.value.as_slice()).unwrap();
        }
        let mut opts = rocksdb::WriteOptions::new();
        opts.set_sync(!self.opts.bulk_import);
        opts.disable_wal(self.opts.bulk_import);
        self.db.write_opt(batch, &opts).unwrap();
    }

    fn flush(&self) {
        let mut opts = rocksdb::WriteOptions::new();
        opts.set_sync(true);
        opts.disable_wal(false);
        let empty = rocksdb::WriteBatch::default();
        self.db.write_opt(empty, &opts).unwrap();
    }
}

impl Drop for DBStore {
    fn drop(&mut self) {
        trace!("closing DB at {:?}", self.opts.path);
    }
}
