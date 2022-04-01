use std::{
    collections::HashSet,
    io::BufReader,
    sync::{
        atomic::{AtomicU32, Ordering},
        mpsc,
    },
    thread::{self, sleep},
    time::Duration,
};

use directories::UserDirs;
use fileperson::{load, FsNode, State};
use rayon::prelude::*;

fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let (_stream, handle) = rodio::OutputStream::try_default().unwrap();
    //let sink = rodio::Sink::try_new(&handle).unwrap();

    let ud = UserDirs::new().unwrap();
    let desktop = ud.desktop_dir().unwrap().to_string_lossy().to_string();
    let arg = std::env::args().nth(1);
    let root = arg.unwrap_or(desktop);
    let mut include = HashSet::new();
    include.extend(
        // ["psd", "DS_Store", "doc", "pdf", "zip", "iso", "eps"]
        ["mp3", "wav", "caf", "aif", "aiff"].map(|s| s.to_lowercase()),
    );
    let (_root, files) = load(root, &include)?;
    let count = AtomicU32::new(0);
    files.entries().par_iter().for_each(|path| {
        if let FsNode::File(path) = path {
            if let Ok(file) = std::fs::File::open(path) {
                let quick = true;
                if quick {
                    if rodio::Decoder::new(BufReader::new(file)).is_ok() {
                        // log::info!("found one: {:?}", path);
                        // sink.append(decoder);
                    } else {
                    }
                    let val = count.fetch_add(1, Ordering::Relaxed);
                    if val % 50 == 0 {
                        log::info!("{val}");
                    }
                } else {
                    match handle.play_once(BufReader::new(file)) {
                        Ok(h) => {
                            log::debug!("2> {:?}", path);
                            let (tx, rx) = mpsc::channel();

                            h.set_volume(0.2);
                            thread::spawn(move || {
                                sleep(Duration::from_millis(500));
                                tx.send(());
                            });
                            rx.recv().ok();
                            println!("! {:?}", path);
                            h.stop();
                        }
                        Err(e) => println!("X_X {:?}", e),
                    }
                }
            }
        }
    });
    // sink.sleep_until_end();
    Ok(())
}
