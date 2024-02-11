// importing common module.
mod common;

#[test]
fn test_rtp() -> anyhow::Result<()> {
    // using common code.
    common::setup()?;

    Ok(())
}
