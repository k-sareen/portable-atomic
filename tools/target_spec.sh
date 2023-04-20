#!/usr/bin/env bash
# shellcheck disable=SC2207
set -euo pipefail
IFS=$'\n\t'
cd "$(dirname "$0")"/..

# shellcheck disable=SC2154
trap 's=$?; echo >&2 "$0: Error on line "${LINENO}": ${BASH_COMMAND}"; exit ${s}' ERR

# Generates types used by codegen.
#
# USAGE:
#    ./tools/target_spec.sh
#
# This script is intended to called by gen.sh.

file="tools/codegen/src/gen/target_spec.rs"
mkdir -p "$(dirname "${file}")"

target_arch=(
    # Architectures that do not included in builtin targets.
    # See also https://github.com/rust-lang/rust/blob/1.69.0/compiler/rustc_target/src/abi/call/mod.rs#L663
    # and https://github.com/rust-lang/rust/blob/540a50df0fb23127edf0b35b0e497748e24bba1a/src/bootstrap/lib.rs#L132.
    amdgpu
    asmjs
    nvptx
    spirv
    xtensa
)
target_os=()
target_env=(
    # Environments that do not included in builtin targets.
    # See also https://github.com/rust-lang/rust/blob/540a50df0fb23127edf0b35b0e497748e24bba1a/src/bootstrap/lib.rs#L130.
    libnx
)
for target in $(rustc --print target-list); do
    target_spec=$(rustc --print target-spec-json -Z unstable-options --target "${target}")
    target_arch+=("$(jq <<<"${target_spec}" -r '.arch')")
    os=$(jq <<<"${target_spec}" -r '.os')
    if [[ "${os}" == "null" ]]; then
        os=none
    fi
    target_os+=("${os}")
    env=$(jq <<<"${target_spec}" -r '.env')
    if [[ "${env}" == "null" ]]; then
        env=none
    fi
    target_env+=("${env}")
done
# sort and dedup
IFS=$'\n'
target_arch=($(LC_ALL=C sort -u <<<"${target_arch[*]}"))
target_os=($(LC_ALL=C sort -u <<<"${target_os[*]}"))
target_env=($(LC_ALL=C sort -u <<<"${target_env[*]}"))
IFS=$'\n\t'

derive='#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]'
default() {
    cat <<EOF
impl Default for $1 {
    fn default() -> Self {
        Self::$2
    }
}
EOF
}
display() {
    cat <<EOF
impl core::fmt::Display for $1 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}
EOF
}
as_str_arm() {
    echo -n "            Self::$1 => \"$1\","
}

cat >"${file}" <<EOF
// This file is @generated by $(basename "$0").
// It is not intended for manual editing.

#![allow(non_camel_case_types)]

use serde::Deserialize;

${derive}
pub enum TargetArch {
$(sed <<<"${target_arch[*]}" -E 's/^/    /g; s/$/,/g')
}
pub use TargetArch::*;
impl TargetArch {
    pub fn as_str(self) -> &'static str {
        match self {
EOF
for arch in "${target_arch[@]}"; do
    as_str_arm "${arch}" >>"${file}"
    echo >>"${file}"
done
cat >>"${file}" <<EOF
        }
    }
}
$(display TargetArch)

${derive}
pub enum TargetOs {
$(sed <<<"${target_os[*]}" -E 's/^/    /g; s/$/,/g')
}
pub use TargetOs::*;
impl TargetOs {
    pub fn as_str(self) -> &'static str {
        match self {
EOF
for os in "${target_os[@]}"; do
    as_str_arm "${os}" >>"${file}"
    echo >>"${file}"
done
cat >>"${file}" <<EOF
        }
    }
}
$(default TargetOs none)
$(display TargetOs)

${derive}
pub enum TargetEnv {
$(sed <<<"${target_env[*]}" -E 's/^/    /g; s/$/,/g')
}
pub use TargetEnv::*;
impl TargetEnv {
    pub fn as_str(self) -> &'static str {
        match self {
EOF
for env in "${target_env[@]}"; do
    if [[ "${env}" == "none" ]]; then
        echo '            Self::none => "",' >>"${file}"
    else
        as_str_arm "${env}" >>"${file}"
        echo >>"${file}"
    fi
done
cat >>"${file}" <<EOF
        }
    }
}
$(default TargetEnv none)
$(display TargetEnv)

${derive}
pub enum TargetEndian {
    big,
    little,
}
pub use TargetEndian::*;
impl TargetEndian {
    pub fn as_str(self) -> &'static str {
        match self {
$(as_str_arm big)
$(as_str_arm little)
        }
    }
}
$(default TargetEndian little)
$(display TargetEndian)
EOF
