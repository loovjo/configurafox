#[allow(unused)]
use tracing::{trace, debug, info, warn, error, instrument, Level};

use std::path::{Path, PathBuf};
use std::io::{Read, Write};

use html_editor::{operation::{Htmlifiable, Editable}, HTMLParseError};

pub mod resource_manager;
pub mod treewalker;

use resource_manager::{Resource, ResourceManager};
use treewalker::{Context, TreeWalker, walk};

#[allow(unused)]
#[derive(Debug)]
pub enum ConfigurafoxError {
    MalformedAttrs { key_name: String, msg: String, },
    MissingAttr { key_name: String, msg: String, },
    MissingBody { msg: String, },
    ParseHTMLError { path: PathBuf, error: HTMLParseError },
    IO(std::io::Error),
    SyntectError(syntect::Error),
    Other(String),
}

impl From<syntect::Error> for ConfigurafoxError {
    fn from(v: syntect::Error) -> Self {
        Self::SyntectError(v)
    }
}

impl From<std::io::Error> for ConfigurafoxError {
    fn from(v: std::io::Error) -> Self {
        Self::IO(v)
    }
}

pub trait ResourceProcessor<R: Resource> {
    fn name(&self) -> String;

    /// Returns the contents of the output file
    fn process_resource(
        &self,
        source: &R,
        source_path: &Path,
        resources: &ResourceManager<R>
    ) -> Result<Vec<u8>, ConfigurafoxError>;
}

pub fn run<'data, R: Resource, D, F: Fn(&Path, &R, &'data D) -> Box<dyn ResourceProcessor<R> + 'data>>(
    output_path: &Path,
    resman: &ResourceManager<R>,
    processor_for: F,
    data: &'data D,
) -> Result<(), ConfigurafoxError> {

    for (resource, path) in resman.all_registered_files() {
        let processor = processor_for(&path, &resource, data);

        info!("Processing {} @ {} w/ {}", resource.identifier(), path.display(), processor.name());

        let processed = processor.process_resource(
            &resource,
            &path,
            resman,
        )?;

        let output_path = {
            let mut output_path = output_path.to_owned();
            output_path.push(resource.output_path());
            output_path
        };

        let output_dir = output_path.parent().expect("No parent dir to output path"); // should never happen as output_path was created with a push
        if !output_dir.exists() {
            debug!("Creating output directory {}", output_dir.display());
            std::fs::create_dir_all(output_dir)?;
        }

        debug!("Writing {} bytes to {}", processed.len(), output_path.display());

        let mut f = std::fs::File::create(output_path)?;
        f.write_all(&processed)?;
    }

    Ok(())
}

/// A do-nothing handler, copying the input to the output verbatim
pub struct IdentityProcessor;

impl<R: Resource> ResourceProcessor<R> for IdentityProcessor {
    fn name(&self) -> String {
        "IdentityHandler".to_string()
    }

    /// Source path is relative to project root
    fn process_resource(
        &self,
        source: &R,
        source_path: &Path,
        resources: &ResourceManager<R>
    ) -> Result<Vec<u8>, ConfigurafoxError> {
        debug!("Copying {}", source.identifier());

        let mut file = std::fs::File::open(resources.absolute_path(source_path))?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;

        Ok(data)
    }
}

/// TODO: Add an image-compressor thingy or something

pub struct HTMLProcessor<'data, R: Resource, D> {
    pub walkers: Vec<Box<dyn TreeWalker<R, D>>>,
    pub trim: bool,
    pub data: &'data D,
}

impl<'data, R: Resource, D> ResourceProcessor<R> for HTMLProcessor<'data, R, D> {
    fn name(&self) -> String {
        let walkers = self.walkers.iter().map(|x| x.describe()).collect::<Vec<_>>().join(", ");
        format!("HTMLProcessor({})", walkers)
    }

    fn process_resource(
        &self,
        source: &R,
        source_path: &Path,
        resources: &ResourceManager<R>
    ) -> Result<Vec<u8>, ConfigurafoxError> {
        debug!("Loading {}", source.identifier());

        let mut file = std::fs::File::open(resources.absolute_path(&source_path))?;
        let mut data = String::new();
        file.read_to_string(&mut data)?;

        let mut dom = html_editor::parse(&data).map_err(|e| ConfigurafoxError::ParseHTMLError { path: source_path.to_owned(), error: e })?;

        let ctx = Context {
            resource: source,
            source_path,
            data: self.data,
            resources,
        };

        walk(
            &mut dom,
            &self.walkers,
            ctx,
        )?;

        if self.trim {
            dom.trim();
        }

        let html_str = dom.html();

        Ok(html_str.into_bytes())
    }
}
