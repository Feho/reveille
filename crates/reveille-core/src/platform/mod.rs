// SPDX-License-Identifier: GPL-2.0-only

//! Platform-neutral pieces of Windows installation discovery and `OpenMoHAA` maintenance.
//!
//! Registry enumeration, process inspection, client launch, and log tailing intentionally remain
//! in the later Windows-only layer.

pub mod gog;
pub mod openmohaa;
pub mod registry;
