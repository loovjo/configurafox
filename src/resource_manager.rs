#[allow(unused)]
use tracing::{trace, debug, info, warn, error, instrument, Level};

use std::hash::Hash;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// enum Resource {
//     HTMLFile(PathBuf),
//     BlogPost(PathBuf),
//
//     Static(PathBuf),
//     Image(PathBuf),
// }

pub trait Resource: Eq + Hash + Clone + std::fmt::Debug {
    /// A "name" to identify this file by
    /// MUST be deterministic
    fn identifier(&self) -> String;

    /// Where should the resulting file be put after generation? Relative to project root
    fn output_path(&self) -> PathBuf;
}

/// Holds all resources, along with some user-specified extra data
pub struct ResourceManager<R: Resource> {
    project_root: PathBuf,

    registered_resources: HashMap<PathBuf, R>,
}

impl<R: Resource> ResourceManager<R> {
    pub fn new(project_root: PathBuf) -> ResourceManager<R> {
        ResourceManager {
            project_root,

            registered_resources: HashMap::new(),
        }
    }

    pub fn absolute_path<P: AsRef<Path>>(&self, path_fragment: P) -> PathBuf {
        let mut res = self.project_root.clone();
        res.push(path_fragment);
        res
    }

    pub fn register_all_files_in_directory<F: Fn(&Path) -> Option<R>>(
        &mut self,
        dir_path: PathBuf,
        parse_resource: F,
        recurse: bool,
    ) -> std::io::Result<()> {
        debug!("Adding files in {}", dir_path.display());
        self.register_all_files_in_directory_ref(dir_path, &parse_resource, recurse)
    }

    fn register_all_files_in_directory_ref<F: Fn(&Path) -> Option<R>>(
        &mut self,
        dir_path: PathBuf,
        parse_resource: &F,
        recurse: bool,
    ) -> std::io::Result<()> {
        for dir_entry in std::fs::read_dir(self.absolute_path(&dir_path))? {
            let dir_entry = dir_entry?;
            let entry_name = dir_entry.file_name();

            let entry_path = {
                if dir_path == PathBuf::from(".") {
                    PathBuf::from(entry_name)
                } else {
                    let mut entry_relative = dir_path.clone();
                    entry_relative.push(entry_name);
                    entry_relative
                }
            };

            let file_type = dir_entry.file_type()?;
            if file_type.is_dir() {
                if recurse {
                    self.register_all_files_in_directory_ref(entry_path.clone(), parse_resource, recurse)?;
                }
            } else {
                let Some(res) = parse_resource(&dir_entry.path()) else {
                    debug!("{}: Not adding", entry_path.display());
                    continue;
                };
                info!("{}: Adding {:?}", entry_path.display(), res.identifier());

                self.registered_resources.insert(entry_path, res);
            }
        }

        Ok(())
    }

    pub fn resource_by_identifier(&self, identifier: &str) -> Option<&R> {
        self.registered_resources
            .values()
            .find(|r| r.identifier() == identifier)

    }

    pub fn all_registered_files(&self) -> HashMap<PathBuf, R> {
        self.registered_resources.clone()
    }
}

