// importing common module.
mod common;

#[test]
fn test_datachannel() -> anyhow::Result<()> {
    // using common code.
    common::setup()?;

    Ok(())
}
