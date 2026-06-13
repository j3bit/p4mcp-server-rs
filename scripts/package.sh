#!/usr/bin/env bash
set -euo pipefail

name="p4-mcp-server"
version="$(cargo pkgid | sed 's/.*#//')"
target_root="${CARGO_TARGET_DIR:-target}"
target_dir="${target_root}/release"
package_root="${target_root}/package"
archive_dir="${package_root}/${name}-${version}"
archive_name="${name}-${version}-$(uname -s)-$(uname -m).tgz"

cargo build --release --locked
rm -rf "${archive_dir}"
mkdir -p "${archive_dir}"

cp "${target_dir}/${name}" "${archive_dir}/"
cp README.md "${archive_dir}/"
mkdir -p "${archive_dir}/docs"
cp docs/offline-build.md "${archive_dir}/docs/"
if [[ -f LICENSE.txt ]]; then
    cp LICENSE.txt "${archive_dir}/"
fi

tar -C "${package_root}" -czf "${package_root}/${archive_name}" "${name}-${version}"
printf '%s\n' "${package_root}/${archive_name}"
