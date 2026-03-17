use crate::RuntimeError;
use crate::utils::{create_dir_all, default_bundle_id, write_string};
use std::path::Path;
use tera::Context;

const DEFAULT_AGENT_YAML: &str = include_str!("../../configs/default_agent.yaml");
const DEFAULT_BUNDLE_MANIFEST: &str = include_str!("../../configs/odyssey.bundle.json5");
const DEFAULT_BUNDLE_README: &str = include_str!("../../configs/README.md");

pub(crate) fn initialize_bundle(root: &Path) -> Result<(), RuntimeError> {
    let bundle_id = default_bundle_id(root);
    let bundle_path = root.display().to_string();
    let context = bundle_template_context(&bundle_id, &bundle_path);

    create_dir_all(&root.join("skills"))?;
    create_dir_all(&root.join("resources"))?;
    write_string(
        &root.join("odyssey.bundle.json5"),
        &render_template(DEFAULT_BUNDLE_MANIFEST, &context)?,
    )?;
    write_string(
        &root.join("agent.yaml"),
        &render_template(DEFAULT_AGENT_YAML, &context)?,
    )?;
    write_string(
        &root.join("README.md"),
        &render_template(DEFAULT_BUNDLE_README, &context)?,
    )?;
    Ok(())
}

fn bundle_template_context(bundle_id: &str, bundle_path: &str) -> Context {
    let mut context = Context::new();
    context.insert("bundle_id", bundle_id);
    context.insert("bundle_path", bundle_path);
    context
}

fn render_template(template: &str, context: &Context) -> Result<String, RuntimeError> {
    tera::Tera::one_off(template, context, false)
        .map_err(|err| RuntimeError::Template(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::initialize_bundle;
    use std::fs;

    #[test]
    fn init_renders_templates_from_target_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("My Cool Bundle");

        initialize_bundle(&root).expect("init");

        let manifest =
            fs::read_to_string(root.join("odyssey.bundle.json5")).expect("read manifest");
        let agent = fs::read_to_string(root.join("agent.yaml")).expect("read agent");
        let readme = fs::read_to_string(root.join("README.md")).expect("read readme");

        assert!(root.join("skills").is_dir());
        assert!(root.join("resources").is_dir());
        assert!(manifest.contains("id: 'my-cool-bundle'"));
        assert!(agent.contains("id: my-cool-bundle"));
        assert!(readme.contains("# my-cool-bundle"));
        assert!(readme.contains(&root.display().to_string()));
    }
}
