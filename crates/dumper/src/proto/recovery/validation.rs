use anyhow::Context;

pub(crate) fn validate_source(source: &str) -> anyhow::Result<()> {
    use protox::file::{File, FileResolver, GoogleFileResolver};
    struct Source(String);
    impl FileResolver for Source {
        fn open_file(&self, name: &str) -> Result<File, protox::Error> {
            if name == "StarRail.proto" {
                File::from_source(name, &self.0)
            } else {
                GoogleFileResolver::new().open_file(name)
            }
        }
    }
    protox::Compiler::with_file_resolver(Source(source.to_owned()))
        .open_file("StarRail.proto")
        .context("validating rendered NT protocol names and references")?;
    Ok(())
}
