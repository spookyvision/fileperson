use caseless::default_case_fold_str;
use itertools::Itertools;
use serde::Deserialize;
use serde::Serialize;
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

#[derive(Serialize, Deserialize)]

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

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash)]
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
    root: PathBuf,
    #[serde(skip)]
    files: Vec<PathBuf>,
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

impl State {
    pub fn new<R: AsRef<Path>>(root: R) -> Self {
        let root = root.as_ref();
        let (files, dirs) = Self::files(root);

        Self {
            root: root.to_path_buf(),
            files,
            infos: HashSet::new(),
        }
    }

    pub fn files<R: AsRef<Path>>(root: R) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let mut files = vec![];
        let mut dirs = vec![];
        let root = root.as_ref();
        for entry in WalkDir::new(root).min_depth(1).sort_by(|a, b| {
            natord::compare_ignore_case(
                a.file_name().to_string_lossy().borrow(),
                b.file_name().to_string_lossy().borrow(),
            )
        }) {
            match entry {
                Ok(entry) => match entry.path().strip_prefix(root) {
                    Ok(path) => {
                        log::info!("{:?}", path);
                        let p = entry.path();
                        if p.is_dir() {
                            dirs.push(p.to_owned())
                        } else {
                            p.parent();
                            files.push(p.to_owned())
                        }
                    }
                    Err(e) => log::warn!("load error: {:?}", e),
                },
                Err(e) => {
                    log::warn!("load error: {:?}", e)
                }
            }
        }

        (files, dirs)
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
        let mut state = State::new(root);

        for f in state.files.clone() {
            let mut fi = FileInfo::from(&f);
            fi.tags = chain.take(rng.gen_range(1..4)).map(|s| s.into()).collect();
            state.add(fi);
        }
        println!("{:?}", state.tags().join(" "));
        Ok(())
    }
}
