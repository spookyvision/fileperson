use caseless::default_case_fold_str;
use itertools::Itertools;
use log::debug;
use rayon::prelude::*;
use serde::Deserialize;
use serde::Serialize;
use std::fs::File;
use std::path::Component;
use std::path::Components;
use std::sync::atomic::AtomicU32;
use std::{
    borrow::{Borrow, Cow},
    collections::{BTreeSet, HashMap, HashSet},
    convert::Infallible,
    fmt::Display,
    fs::DirEntry,
    hash::Hash,
    iter::Filter,
    path::{Path, PathBuf},
    rc::Rc,
    str::FromStr,
};
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
    path: PathBuf,
    delete: Option<bool>,
    tags: Vec<TagRef>,
}

impl<P: AsRef<Path>> From<P> for FileInfo {
    fn from(p: P) -> Self {
        Self {
            path: p.as_ref().to_path_buf(),
            delete: None,
            tags: vec![],
        }
    }
}

impl Borrow<Path> for FileInfo {
    fn borrow(&self) -> &Path {
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
    this: PathBuf,
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
    File(PathBuf),
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

fn load_rec(
    parent: &mut Directory,
    flat: &mut Directory,
    include: &HashSet<String>,
    count: &AtomicU32,
) {
    let parent_as_path = &parent.this;
    for entry in WalkDir::new(parent_as_path)
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
            log::info!("(load) {val}");
        }
        match entry {
            Ok(entry) => match entry.path().strip_prefix(parent_as_path) {
                Ok(path) => {
                    let p = entry.path();
                    let node = if p.is_dir() {
                        let cs = p.components();
                        let mut dir = Directory {
                            this: p.into(),
                            entries: vec![],
                        };
                        load_rec(&mut dir, flat, include, count);
                        Some(FsNode::Directory(dir))
                    } else if p.is_file() {
                        if let Some(name) = p.file_name() {
                            if let Some(extension) = name.to_string_lossy().split(".").last() {
                                if !include.contains(&extension.to_lowercase()) {
                                    // log::warn!("includeping {name:?}");
                                    continue;
                                }
                            }
                        }
                        let node = FsNode::File(p.into());
                        flat.entries.push(node.clone());
                        Some(node)
                    } else {
                        None
                    };

                    if let Some(node) = node {
                        parent.entries.push(node);
                    }
                    // node_root.entries.push(FsNode::from(p));
                }
                Err(e) => log::warn!("load error: {:?}", e),
            },
            Err(e) => {
                log::warn!("load error: {:?}", e)
            }
        }
    }
}
pub fn load<R: AsRef<Path>>(
    root: R,
    include: &HashSet<String>,
) -> anyhow::Result<(Directory, Directory)> {
    let root = root.as_ref();

    let mut node_root = Directory {
        this: root.to_owned(),
        entries: vec![],
    };

    let mut flat = node_root.clone();
    let count = AtomicU32::new(0);
    load_rec(&mut node_root, &mut flat, include, &count);

    Ok((node_root, flat))
}

impl State {
    pub fn new<R: AsRef<Path>>(root: R) -> anyhow::Result<Self> {
        let root = root.as_ref();

        let mut include = HashSet::new();
        include.insert("psd".into());
        let (root, flat) = load(root, &include)?;
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
        let root = UserDirs::new().unwrap();
        let root = root.desktop_dir().unwrap();
        let mut state = State::new(root)?;

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
