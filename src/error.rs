use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("LED node does not exist: {0}")]
    LedNodeMissing(PathBuf),

    #[error("LED node is not writable: {path}: {source}")]
    LedNodeNotWritable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "no LED matched selector vendor={vendor} product={product} name={name} \
         under /sys/class/leds"
    )]
    LedSelectorNotFound {
        vendor: String,
        product: String,
        name: String,
    },

    #[error(
        "wrote {wrote:?} to {path} but read back {read_back:?} -- the write did not take \
         effect. On input-device LEDs this happens when another process holds an exclusive \
         EVIOCGRAB on the backing /dev/input node; check `fuser -v` on it"
    )]
    WriteVerifyMismatch {
        path: PathBuf,
        wrote: String,
        read_back: String,
    },

    #[error("no watched block devices found (configured devices: {configured:?})")]
    NoBlockDevicesFound { configured: Vec<String> },

    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid config at {path}: {source}")]
    Config {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}
