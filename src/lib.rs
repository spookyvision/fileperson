use std::{
    borrow::{Borrow, Cow},
    collections::{BTreeSet, HashMap, HashSet},
    convert::Infallible,
    error::Error,
    fmt::Display,
    fs::{DirEntry, File},
    hash::Hash,
    iter::Filter,
    path::{Component, Components, StripPrefixError},
    rc::Rc,
    str::FromStr,
    sync::atomic::AtomicU32,
};

use camino::{Utf8Path, Utf8PathBuf};
use caseless::default_case_fold_str;
use itertools::Itertools;
use log::{debug, error, info};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use walkdir::WalkDir;

#[derive(Serialize, Deserialize, Clone)]

pub struct Tag {
    color: Option<String>,
    value: String,
}

impl FromStr for Tag {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            color: None,
            value: s.to_string(),
        })
    }
}

impl From<&str> for Tag {
    fn from(s: &str) -> Self {
        Tag::from_str(s).unwrap()
    }
}

impl Display for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.value)
    }
}

impl Hash for Tag {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        caseless::default_case_fold_str(&self.value).hash(state);
    }
}

impl PartialEq for Tag {
    fn eq(&self, other: &Self) -> bool {
        caseless::default_caseless_match_str(&self.value, &other.value)
    }
}

impl Eq for Tag {}

impl Ord for Tag {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        caseless::default_case_fold_str(&self.value)
            .cmp(&caseless::default_case_fold_str(&other.value))
    }
}
impl PartialOrd for Tag {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

type TagRef = Tag;

struct Action<'a> {
    file: &'a FileInfo,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash, Clone)]
pub struct FileInfo {
    path: Utf8PathBuf,
    delete: Option<bool>,
    tags: Vec<TagRef>,
}

impl<P: AsRef<Utf8Path>> From<P> for FileInfo {
    fn from(p: P) -> Self {
        Self {
            path: p.as_ref().to_path_buf(),
            delete: None,
            tags: vec![],
        }
    }
}

impl Borrow<Utf8Path> for FileInfo {
    fn borrow(&self) -> &Utf8Path {
        self.path.as_path()
    }
}

impl FileInfo {
    pub fn touched(&self) -> bool {
        self.delete.is_some() || !self.tags.is_empty()
    }

    pub fn set_tags(&mut self, tags: Vec<TagRef>) {
        if self.delete.is_none() {
            self.delete = Some(true);
        }
    }

    pub fn tags(&self) -> &Vec<TagRef> {
        &self.tags
    }

    // a file marked for deletion that has tags should raise a warning
    pub fn questionable_state(&self) -> bool {
        self.delete.unwrap_or(false) && !self.tags.is_empty()
    }
}

#[derive(Serialize, Deserialize)]
pub struct State {
    root: Directory,
    flat: Directory,
    infos: HashSet<FileInfo>,
}

trait DirEntryExt {
    fn file_name_lossy(&self) -> String;
}

impl DirEntryExt for DirEntry {
    fn file_name_lossy(&self) -> String {
        self.file_name().to_string_lossy().to_string()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Directory {
    this: Utf8PathBuf,
    entries: Vec<FsNode>,
}

impl Directory {
    /// Get a reference to the directory's entries.
    #[must_use]
    pub fn entries(&self) -> &[FsNode] {
        self.entries.as_ref()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum FsNode {
    File(Utf8PathBuf),
    Directory(Directory),
}

impl FsNode {
    fn entry_iter(&mut self, components: Components) {
        for comp in components {
            self.entry(comp);
        }
    }
    fn entry(&self, comp: Component) -> Option<FsNode> {
        if let Component::Normal(n) = comp {
            match self {
                FsNode::File(_) => todo!(),
                FsNode::Directory(_) => todo!(),
            }
        }
        None
    }
}

#[derive(Error, Debug)]
enum LoadError {
    #[error("Walk")]
    Walk(#[from] walkdir::Error),
    #[error("Strip")]
    Strip(#[from] StripPrefixError),
    #[error("Bork")]
    NonUtf8Path(PathBuf),
}

use std::path::PathBuf;
fn load_rec(
    parent: &mut Directory,
    flat: &mut Directory,
    include: &HashSet<String>,
    count: &AtomicU32,
) {
    let parent_as_path = parent.this.clone();
    for entry in WalkDir::new(parent_as_path.clone())
        .min_depth(1)
        .max_depth(1)
        .sort_by(|a, b| {
            natord::compare_ignore_case(
                a.file_name().to_string_lossy().borrow(),
                b.file_name().to_string_lossy().borrow(),
            )
        })
    {
        let val = count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if val % 100 == 0 {
            info!("(load) {val}");
        }

        let maybe_path = entry.map_err(LoadError::from).and_then(|entry| {
            entry
                .path()
                .strip_prefix(parent_as_path.clone())
                .map_err(LoadError::from)
                .and_then(|_path| {
                    Utf8PathBuf::from_path_buf(entry.path().to_owned())
                        .map_err(|e| LoadError::NonUtf8Path(e))
                })
                .and_then(|path| {
                    if path.is_dir() {
                        let cs = path.components();
                        let mut dir = Directory {
                            this: path,
                            entries: vec![],
                        };
                        load_rec(&mut dir, flat, include, count);
                        parent.entries.push(FsNode::Directory(dir));
                    } else if path.is_file() {
                        if let Some(name) = path.file_name() {
                            if let Some(extension) = name.split(".").last() {
                                if !include.contains(&extension.to_lowercase()) {
                                    // log::warn!("includeping {name:?}");
                                }
                            }
                        }
                        let node = FsNode::File(path.into());
                        flat.entries.push(node.clone());
                        parent.entries.push(node);
                    } else {
                        log::debug!("skipping {path:?}");
                    };

                    Ok(())
                })
        });

        if let Err(e) = maybe_path {
            error!("{e:?}")
        }
    }
}
pub fn load(
    root: impl AsRef<Utf8Path>,
    include: HashSet<impl AsRef<str>>,
) -> anyhow::Result<(Directory, Directory)> {
    let root = root.as_ref();

    let mut node_root = Directory {
        this: root.to_owned(),
        entries: vec![],
    };

    let mut flat = node_root.clone();
    let count = AtomicU32::new(0);
    load_rec(
        &mut node_root,
        &mut flat,
        &(include
            .into_iter()
            .map(|s| s.as_ref().to_lowercase())
            .collect()),
        &count,
    );

    Ok((node_root, flat))
}

impl State {
    pub fn new(
        root: impl AsRef<Utf8Path>,
        include: HashSet<impl AsRef<str>>,
    ) -> anyhow::Result<Self> {
        let root = root.as_ref();

        let (root, flat) = load(root, include)?;
        Ok(Self {
            root,
            flat,
            infos: HashSet::new(),
        })
    }

    pub fn tags_filter<P: FnMut(&&FileInfo) -> bool>(
        &self,
        predicate: P,
    ) -> impl Iterator<Item = &Tag> {
        self.infos
            .iter()
            .filter(predicate)
            .flat_map(|f| f.tags())
            .sorted()
            .dedup()
    }

    pub fn tags(&self) -> impl Iterator<Item = &Tag> {
        self.infos.iter().flat_map(|f| f.tags()).sorted().dedup()
    }

    pub fn add(&mut self, f: FileInfo) -> anyhow::Result<()> {
        // let tag = caseless::default_case_fold_str("s");
        // let mut f = std::fs::File::open("/tmp/test.txt")?;
        self.infos.insert(f);
        Ok(())
    }
}

impl Extend<FileInfo> for State {
    fn extend<T: IntoIterator<Item = FileInfo>>(&mut self, iter: T) {
        self.infos.extend(iter);
    }
}
#[cfg(test)]
mod tests {

    use anyhow::anyhow;
    use directories::UserDirs;
    use lipsum::{MarkovChain, LIBER_PRIMUS, LOREM_IPSUM};
    use rand::{prelude::SliceRandom, Rng};

    use super::*;
    #[test]
    fn test_rc() {}

    #[test]
    fn test_tags() -> anyhow::Result<()> {
        let mut rng = rand::thread_rng();
        let mut chain = MarkovChain::new();
        chain.learn(LOREM_IPSUM);
        chain.learn(LIBER_PRIMUS);
        let chain = &mut chain.iter();
        let root = UserDirs::new().ok_or(anyhow!("no UserDirs"))?;
        let root = root.desktop_dir().ok_or(anyhow!("no Desktop dir"))?;
        let root =
            Utf8Path::from_path(root).ok_or(anyhow!("Desktop dir is not utf-8: {root:?}"))?;
        let mut state = State::new(root, HashSet::from(["mp3", "wav"]))?;

        for f in state.root.entries.clone() {
            if let FsNode::File(f) = f {
                let mut fi = FileInfo::from(f);
                fi.tags = chain.take(rng.gen_range(1..4)).map(|s| s.into()).collect();
                state.infos.insert(fi);
            }
        }

        println!("{:?}", state.tags().join(" "));
        Ok(())
    }
}
