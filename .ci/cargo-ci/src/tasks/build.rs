use std::path::PathBuf;

use crate::{constants, Package};
use crate::core::commands::CROSS;
use crate::types::Arch;
use crate::core::types::parsing::target::TargetSelection;
use crate::util::RunRequiringSuccess;


/// Perform a release build, without bundling a distribution.
#[derive(Debug, clap::Parser)]
#[command(hide=true)]
pub struct DistributionBuildCli {
    #[arg(long, default_value_t)]
    pub target: TargetSelection,
}

#[tracing::instrument]
pub fn distribution_build(package: Package, target: Arch) -> crate::Result {
    CROSS.command()
        .args([
            "--package",
            &package.ident(),
            "--target-dir",
            &cross_target_dir().display().to_string(), //explicitly set target-base-dir to fix unreliable caching behavior
            "--target",
            &target.triple(),
        ])
        .run_requiring_success()?;
    Ok(())
}

pub fn out_dir(package: Package, target: Arch) -> PathBuf {
    cross_target_dir().join(target.triple()).join("release").join(package.ident())
}

fn cross_target_dir() -> PathBuf {
    constants::target_dir().join("cross")
}
