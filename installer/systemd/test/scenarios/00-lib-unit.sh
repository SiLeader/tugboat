#!/usr/bin/env bash
# shellcheck disable=SC1091,SC2016
set -euo pipefail

SCENARIO_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
INSTALLER_DIR="$(cd -- "${SCENARIO_DIR}/../.." && pwd -P)"
REPO_ROOT="$(cd -- "${INSTALLER_DIR}/../.." && pwd -P)"
TMP_DIR="$(mktemp -d -t tugboat-lib-test.XXXXXXXXXX)"

cleanup() {
    rm -rf -- "${TMP_DIR}"
}
trap cleanup EXIT

# shellcheck source=../../lib.sh
source "${INSTALLER_DIR}/lib.sh"

fail() {
    printf 'not ok: %s\n' "$*" >&2
    exit 1
}

assert_eq() {
    local expected="$1"
    local actual="$2"
    local message="$3"

    if [[ "${actual}" != "${expected}" ]]; then
        fail "${message}: expected '${expected}', got '${actual}'"
    fi
}

assert_file_contains() {
    local path="$1"
    local needle="$2"

    if ! grep -Fq -- "${needle}" "${path}"; then
        fail "${path} does not contain '${needle}'"
    fi
}

make_mock_bin() {
    local name="$1"
    local body="$2"
    local path="${TMP_DIR}/bin/${name}"

    mkdir -p -- "${TMP_DIR}/bin"
    {
        printf '#!/usr/bin/env bash\n'
        printf 'set -euo pipefail\n'
        printf '%s\n' "${body}"
    } > "${path}"
    chmod +x -- "${path}"
}

test_installer_dir() {
    assert_eq "${INSTALLER_DIR}" "${INSTALLER_DIR}" "INSTALLER_DIR should be set"
    [[ -f "${INSTALLER_DIR}/lib.sh" ]] || fail "INSTALLER_DIR does not point at installer/systemd"
    assert_eq "${REPO_ROOT}" "${CARGO_MANIFEST_DIR}" "CARGO_MANIFEST_DIR should default to repo root"
}

test_logging() {
    local output

    output="$(log_info hello 2>&1 >/dev/null)"
    [[ "${output}" == *"[INFO]"* ]] || fail "log_info should include INFO level"

    output="$(log_warn hello 2>&1 >/dev/null)"
    [[ "${output}" == *"[WARN]"* ]] || fail "log_warn should include WARN level"

    output="$(log_error hello 2>&1 >/dev/null)"
    [[ "${output}" == *"[ERROR]"* ]] || fail "log_error should include ERROR level"
}

test_require_root() {
    if [[ "${EUID}" -eq 0 ]]; then
        require_root
    else
        if (
            # shellcheck source=../../lib.sh
            source "${INSTALLER_DIR}/lib.sh"
            require_root
        ) >/dev/null 2>&1; then
            fail "require_root should fail for non-root users"
        fi
    fi
}

test_parse_build_mode() {
    if parse_build_mode >/dev/null 2>&1; then
        fail "parse_build_mode should require --build or --use-prebuilt"
    fi

    parse_build_mode --build
    assert_eq "build" "${BUILD_MODE}" "explicit build mode"
    assert_eq "" "${PREBUILT_BIN_DIR}" "explicit build prebuilt dir"

    parse_build_mode --use-prebuilt --bin-dir "${TMP_DIR}/prebuilt" --ignored value
    assert_eq "prebuilt" "${BUILD_MODE}" "prebuilt build mode"
    assert_eq "${TMP_DIR}/prebuilt" "${PREBUILT_BIN_DIR}" "prebuilt dir"

    parse_build_mode --use-prebuilt --bin-dir="${TMP_DIR}/prebuilt-equals"
    assert_eq "prebuilt" "${BUILD_MODE}" "prebuilt mode from --bin-dir="
    assert_eq "${TMP_DIR}/prebuilt-equals" "${PREBUILT_BIN_DIR}" "prebuilt dir from --bin-dir="

    if parse_build_mode --use-prebuilt >/dev/null 2>&1; then
        fail "parse_build_mode should reject --use-prebuilt without --bin-dir"
    fi

    if parse_build_mode --build --use-prebuilt --bin-dir "${TMP_DIR}/prebuilt" >/dev/null 2>&1; then
        fail "parse_build_mode should reject conflicting build modes"
    fi

    if parse_build_mode --build --bin-dir "${TMP_DIR}/prebuilt" >/dev/null 2>&1; then
        fail "parse_build_mode should reject --bin-dir with --build"
    fi
}

test_build_binary() {
    local fake_repo="${TMP_DIR}/repo"

    mkdir -p -- "${fake_repo}"
    touch "${fake_repo}/Cargo.toml"
    make_mock_bin "cargo" 'printf "%s\n" "$PWD" > "${MOCK_LOG}/cargo-pwd"; printf "%s\n" "$*" > "${MOCK_LOG}/cargo-args"'

    PATH="${TMP_DIR}/bin:${PATH}" MOCK_LOG="${TMP_DIR}" CARGO_MANIFEST_DIR="${fake_repo}" build_binary tugboat-apiserver
    assert_eq "${fake_repo}" "$(cat "${TMP_DIR}/cargo-pwd")" "build_binary should run from Cargo manifest dir"
    assert_eq "build --release --package tugboat-apiserver" "$(cat "${TMP_DIR}/cargo-args")" "build_binary cargo args"
}

test_install_binary_prebuilt() {
    local prebuilt="${TMP_DIR}/prebuilt"
    local dest="${TMP_DIR}/dest-bin"

    mkdir -p -- "${prebuilt}"
    printf 'binary\n' > "${prebuilt}/tugboat-agent"

    BUILD_MODE="prebuilt"
    PREBUILT_BIN_DIR="${prebuilt}"
    install_binary tugboat-agent "${dest}"

    assert_file_contains "${dest}/tugboat-agent" "binary"
    [[ -x "${dest}/tugboat-agent" ]] || fail "installed binary should be executable"
}

test_install_binary_build() {
    local fake_repo="${TMP_DIR}/repo-build"
    local dest="${TMP_DIR}/dest-build"

    mkdir -p -- "${fake_repo}/target/release"
    printf 'built\n' > "${fake_repo}/target/release/tugboat-scheduler"
    make_mock_bin "cargo" 'exit 0'

    BUILD_MODE="build"
    PATH="${TMP_DIR}/bin:${PATH}" CARGO_MANIFEST_DIR="${fake_repo}" install_binary tugboat-scheduler "${dest}"
    assert_file_contains "${dest}/tugboat-scheduler" "built"
}

test_fetch_tarball() {
    local source_dir="${TMP_DIR}/tar-source"
    local tarball="${TMP_DIR}/fixture.tar.gz"
    local dest="${TMP_DIR}/tar-dest"
    local checksum

    mkdir -p -- "${source_dir}"
    printf 'payload\n' > "${source_dir}/payload.txt"
    tar czf "${tarball}" -C "${source_dir}" payload.txt
    checksum="$(sha256sum "${tarball}" | awk '{print $1}')"
    make_mock_bin "wget" 'if [[ "$1" != "-O" ]]; then exit 2; fi; cp -- "$3" "$2"'

    PATH="${TMP_DIR}/bin:${PATH}" fetch_tarball "${tarball}" "${checksum}" "${dest}"
    assert_file_contains "${dest}/payload.txt" "payload"
}

test_render_template() {
    local src="${TMP_DIR}/template.txt"
    local dst="${TMP_DIR}/rendered/output.txt"

    make_mock_bin "envsubst" 'content="$(cat)"; content="${content//\$\{TEST_VALUE\}/${TEST_VALUE}}"; printf "%s" "${content}"'
    printf 'value=${TEST_VALUE}\n' > "${src}"

    TEST_VALUE="rendered" PATH="${TMP_DIR}/bin:${PATH}" render_template "${src}" "${dst}"
    assert_file_contains "${dst}" "value=rendered"
}

test_create_system_user() {
    local log="${TMP_DIR}/useradd.log"

    make_mock_bin "id" 'if [[ "${2:-}" == "existing-user" ]]; then exit 0; fi; exit 1'
    make_mock_bin "useradd" 'printf "%s\n" "$*" >> "${MOCK_LOG}/useradd.log"'

    PATH="${TMP_DIR}/bin:${PATH}" MOCK_LOG="${TMP_DIR}" create_system_user existing-user
    [[ ! -f "${log}" ]] || fail "create_system_user should not call useradd for existing user"

    PATH="${TMP_DIR}/bin:${PATH}" MOCK_LOG="${TMP_DIR}" create_system_user new-user
    assert_file_contains "${log}" "--system --no-create-home --shell /usr/sbin/nologin new-user"
}

test_install_unit() {
    local src="${TMP_DIR}/unit.tpl"
    local unit_dir="${TMP_DIR}/units"

    make_mock_bin "envsubst" 'content="$(cat)"; content="${content//\$\{UNIT_VALUE\}/${UNIT_VALUE}}"; printf "%s" "${content}"'
    printf '[Service]\nExecStart=${UNIT_VALUE}\n' > "${src}"

    UNIT_VALUE="/bin/true" PATH="${TMP_DIR}/bin:${PATH}" SYSTEMD_UNIT_DIR="${unit_dir}" install_unit "${src}" "example.service"
    assert_file_contains "${unit_dir}/example.service" "ExecStart=/bin/true"
}

test_enable_disable_unit() {
    make_mock_bin "systemctl" 'printf "%s\n" "$*" >> "${MOCK_LOG}/systemctl.log"; if [[ "$1" == "disable" && "${MOCK_DISABLE_FAIL:-0}" == "1" ]]; then exit 1; fi'

    PATH="${TMP_DIR}/bin:${PATH}" MOCK_LOG="${TMP_DIR}" enable_unit example.service
    assert_file_contains "${TMP_DIR}/systemctl.log" "daemon-reload"
    assert_file_contains "${TMP_DIR}/systemctl.log" "enable --now example.service"

    PATH="${TMP_DIR}/bin:${PATH}" MOCK_LOG="${TMP_DIR}" MOCK_DISABLE_FAIL=1 disable_unit missing.service
    assert_file_contains "${TMP_DIR}/systemctl.log" "disable --now missing.service"
}

test_installer_dir
test_logging
test_require_root
test_parse_build_mode
test_build_binary
test_install_binary_prebuilt
test_install_binary_build
test_fetch_tarball
test_render_template
test_create_system_user
test_install_unit
test_enable_disable_unit

printf 'ok: 00-lib-unit\n'
