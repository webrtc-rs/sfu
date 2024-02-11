// importing common module.
mod common;

#[test]
fn test_signaling() -> anyhow::Result<()> {
    // using common code.
    common::setup()?;

    Ok(())
}
