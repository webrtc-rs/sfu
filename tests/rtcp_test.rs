// importing common module.
mod common;

#[ignore]
#[test]
fn test_rtcp() -> anyhow::Result<()> {
    // using common code.
    common::setup()?;

    Ok(())
}
