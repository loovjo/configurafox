#[allow(unused)]
use tracing::{trace, debug, info, warn, error, instrument, Level};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use syntect::{parsing::SyntaxSet, highlighting::ThemeSet, html::highlighted_html_for_string};

use html_editor::{Node, Element};

use crate::{ConfigurafoxError, resource_manager::{Resource, ResourceManager}};

pub fn get_attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find_map(|(k, v)| if k == key { Some(&**v) } else { None} )
}

pub struct Context<'res, 'data, R: Resource, D> {
    pub resource: &'res R,
    pub source_path: &'res Path,
    pub resources: &'res ResourceManager<R>,
    pub data: &'data D,
}

impl<'res, 'data, R: Resource, D> Clone for Context<'res, 'data, R, D> {
    fn clone(&self) -> Self {
        Context {
            resource: self.resource,
            source_path: self.source_path,
            resources: self.resources,
            data: self.data,
        }
    }
}

impl<'res, 'data, R: Resource, D> Copy for Context<'res, 'data, R, D> {}


pub trait TreeWalker<R: Resource, D> {
    fn describe(&self) -> String;

    fn matches(&self, tag_name: &str, attrs: &[(String, String)], ctx: Context<'_, '_, R, D>) -> bool;

    fn replace(&self, tag_name: &str, attrs: Vec<(String, String)>, children: Vec<Node>, ctx: Context<'_, '_, R, D>) -> Result<Vec<Node>, ConfigurafoxError>;
}

pub fn walk<'res, 'data, R: Resource, D>(dom: &mut Vec<Node>, replacers: &[Box<dyn TreeWalker<R, D>>], ctx: Context<'res, 'data, R, D>) -> Result<(), ConfigurafoxError> {
    let original_dom = std::mem::replace(dom, Vec::with_capacity(dom.len()));

    'outer: for el in original_dom {
        let Node::Element(Element { name, attrs, children }) = el else {
            dom.push(el);
            continue;
        };

        for replacer in replacers {
            if replacer.matches(&name, &attrs, ctx) {
                let res = replacer.replace(&name, attrs, children, ctx)?;
                dom.extend(res);
                continue 'outer;
            }
        }

        dom.push(Node::Element(Element { name, attrs, children }));
    }

    for el in dom {
        if let Node::Element(Element { children, .. }) = el {
            walk(children, replacers, ctx)?;
        }
    }

    Ok(())
}

pub struct VariableReplacer(pub HashMap<String, String>);

impl<R: Resource, D> TreeWalker<R, D> for VariableReplacer {
    fn describe(&self) -> String {
        let variables = self.0.iter().map(|(k, v)| format!("{k:?} = {v:?}")).collect::<Vec<_>>().join(", ");

        format!("VariableReplacer({})", variables)
    }

    fn matches(&self, tag_name: &str, attrs: &[(String, String)], _ctx: Context<'_, '_, R, D>) -> bool {
        tag_name.starts_with('$') || attrs.iter().map(|(_k, v)| v).any(|v| v.starts_with("$"))
    }

    fn replace(&self, tag_name: &str, attrs: Vec<(String, String)>, children: Vec<Node>, _ctx: Context<'_, '_, R, D>) -> Result<Vec<Node>, ConfigurafoxError> {
        let replace_var = |x: String| -> Result<String, ConfigurafoxError> {
            if !x.starts_with('$') {
                return Ok(x);
            }
            let Some(var) = self.0.get(&x[1..]) else {
                return Err(ConfigurafoxError::Other(format!("Unknown variable {x}")));
            };
            Ok(var.clone())
        };

        if tag_name.starts_with("$") {
            Ok(vec![Node::Text(replace_var(tag_name.to_owned())?)])
        } else {
            let new_attrs = attrs
                .into_iter()
                .map(|(k, v)| Ok((k, replace_var(v)?)))
                .collect::<Result<Vec<_>, ConfigurafoxError>>()?;

            let new_elem = Node::Element(Element { name: tag_name.to_owned(), attrs: new_attrs, children });
            Ok(vec![new_elem])
        }
    }
}

pub struct LinkReplacer;

impl<R: Resource, D> TreeWalker<R, D> for LinkReplacer {
    fn describe(&self) -> String {
        "LinkReplacer".to_string()
    }

    fn matches(&self, _tag_name: &str, attrs: &[(String, String)], _ctx: Context<'_, '_, R, D>) -> bool {
        attrs.iter().map(|(_k, v)| v).any(|v| v.starts_with("@"))
    }

    fn replace(&self, tag_name: &str, attrs: Vec<(String, String)>, children: Vec<Node>, ctx: Context<'_, '_, R, D>) -> Result<Vec<Node>, ConfigurafoxError> {
        let source_dir = ctx.source_path.parent();

        let replace_link = |x: String| -> Result<String, ConfigurafoxError> {
            if !x.starts_with('@') {
                return Ok(x);
            }
            let identifier = &x[1..];

            for (resource, _) in &ctx.resources.all_registered_files() {
                let path = resource.output_path();
                if resource.identifier() == identifier {
                    let diff = if let Some(source_dir) = source_dir {
                        pathdiff::diff_paths(&path, source_dir)
                            .expect(&format!("Resource referenced ({}) could not be relativized from {}", path.display(), ctx.source_path.display()))
                    } else {
                        path.clone()
                    };

                    debug!("{} - {} = {}", path.display(), ctx.source_path.display(), diff.display());

                    return Ok(diff.to_str().expect("Invalid UTF-8 in path").to_owned());
                }
            }

            Err(ConfigurafoxError::Other(format!("Unknown identifier: {x}")))
        };

        let new_attrs = attrs
            .into_iter()
            .map(|(k, v)| Ok((k, replace_link(v)?)))
            .collect::<Result<Vec<_>, ConfigurafoxError>>()?;

        let new_elem = Node::Element(Element { name: tag_name.to_owned(), attrs: new_attrs, children });
        Ok(vec![new_elem])
    }
}

pub struct KatexReplacer;

impl<R: Resource, D> TreeWalker<R, D> for KatexReplacer {
    fn describe(&self) -> String {
        "KatexReplacer".to_string()
    }

    fn matches(&self, tag_name: &str, _attrs: &[(String, String)], _ctx: Context<'_, '_, R, D>) -> bool {
        tag_name == "$" || tag_name == "katex" || tag_name == "katex-prelude"
    }

    fn replace(&self, tag_name: &str, _attrs: Vec<(String, String)>, children: Vec<Node>, _ctx: Context<'_, '_, R, D>) -> Result<Vec<Node>, ConfigurafoxError> {
        match tag_name {
            "katex-prelude" => {
                Ok(vec![
                    Node::Element(Element {
                        name: "link".into(),
                        attrs: vec![("rel".into(), "stylesheet".into()), ("href".into(), format!("https://cdn.jsdelivr.net/npm/katex@{}/dist/katex.min.css", katex::KATEX_VERSION))],
                        children: vec![]
                    })
                ])
            }
            "katex" | "$" => {
                let mut opts = katex::Opts::builder()
                    .output_type(katex::opts::OutputType::Html)
                    .trust(true)
                    .build()
                    .unwrap();

                if tag_name == "katex" {
                    opts.set_display_mode(true);
                }

                match &children[..] {
                    [Node::Text(tex)] => {
                        let rendered = katex::render_with_opts(tex, &opts).expect("meow");
                        Ok(vec![Node::RawHTML(rendered)])
                    }
                    _ => {
                        Err(ConfigurafoxError::Other("Katex: malformed body".to_string()))
                    }
                }
            }
            _ => unreachable!("invalid tag {tag_name} for KatexReplacer"),
        }
    }
}

fn deindent(source: &str) -> String {
    let source = source.trim_start_matches("\n").trim_end();
    let n_spaces = source.chars().take_while(|&c| c == ' ').count();
    let prefix = std::iter::repeat(' ').take(n_spaces).collect::<String>();

    source
        .lines()
        .map(|x| x.strip_prefix(&prefix).unwrap_or(x).to_string())
        .collect::<Vec<String>>()
        .join("\n")
}

pub struct SyntaxHighlighter {
    pub syntax_set: SyntaxSet,
    pub theme_set: ThemeSet,
    pub theme: String,
}

impl SyntaxHighlighter {
    pub fn default(theme: &str) -> SyntaxHighlighter {
        SyntaxHighlighter {
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
            theme: theme.to_string(),
        }
    }
}

impl<R: Resource, D> TreeWalker<R, D> for SyntaxHighlighter {
    fn describe(&self) -> String {
        "SyntaxHighlighter".to_string()
    }

    fn matches(&self, tag_name: &str, _attrs: &[(String, String)], _ctx: Context<'_, '_, R, D>) -> bool {
        tag_name == "code-hl" || tag_name == "pre-hl"
    }

    fn replace(&self, tag_name: &str, attrs: Vec<(String, String)>, children: Vec<Node>, _ctx: Context<'_, '_, R, D>) -> Result<Vec<Node>, ConfigurafoxError> {
        let code_text = match children.as_slice() {
            [Node::Text(code_text)] => code_text.to_owned(),
            _ => return Err(ConfigurafoxError::Other(format!("{tag_name} must contain only text children"))),
        };
        let code_text = deindent(&code_text);

        let lang = get_attr(&attrs, "lang").ok_or(ConfigurafoxError::Other("Missing lang= attribute".to_string()))?;

        let theme = &self.theme_set.themes.get(&self.theme).ok_or(ConfigurafoxError::Other(format!("No such theme {}", self.theme)))?;

        let background_color_style = theme.settings.background.map(|col| format!("background: #{:02x}{:02x}{:02x};", col.r, col.g, col.b));

        let syntax_reference = self
            .syntax_set
            .find_syntax_by_extension(&lang)
            .ok_or(ConfigurafoxError::Other(format!("Unknown language {lang}")))?;

        let html_str = highlighted_html_for_string(&code_text, &self.syntax_set, syntax_reference, &theme)?;

        let html_parsed = html_editor::parse(&html_str)
            .map_err(|e| ConfigurafoxError::ParseHTMLError { path: PathBuf::from("<generated-syntect>"), error: e })?;


        let Some(Node::Element(Element { name, mut attrs, children })) = html_parsed.into_iter().next() else {
            return Err(ConfigurafoxError::Other(format!("Invalid html generated by syntect: {html_str:?}")));
        };

        if name != "pre" {
            return Err(ConfigurafoxError::Other(format!("Invalid html generated by syntect: {html_str:?}")));
        }


        if let Some(bg_style) = background_color_style {
            attrs.push(("style".to_string(), bg_style));
        }

        match tag_name {
            "pre-hl" => {
                Ok(vec![
                    Node::Element(Element {
                        name: "pre".to_string(),
                        attrs,
                        children,
                    }),
                ])
            }
            "code-hl" => {
                Ok(vec![
                    Node::Element(Element {
                        name: "code".to_string(),
                        attrs,
                        children,
                    }),
                ])
            }
            _ => unreachable!(),
        }
    }
}
