// SPDX-License-Identifier: GPL-2.0-only

//! Platform-neutral pieces of Windows installation discovery and `OpenMoHAA` maintenance.
//!
//! Live registry enumeration is compiled only on Windows. Process inspection, client launch, and
//! install-target policy belong to callers; client log tailing is explicitly outside v1.

pub mod gog;
pub mod openmohaa;
pub mod package;
pub mod reborn;
pub mod registry;
