#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../../.." && pwd -P)"
cd "$REPO_ROOT"

python3 -m py_compile installer/multipass/demo.py
python3 -m unittest discover -s installer/multipass/test -p 'test_*.py'

for playbook in installer/multipass/ansible/*.yml; do
    ANSIBLE_CONFIG=installer/ansible/ansible.cfg ansible-playbook --syntax-check \
        -i installer/ansible/inventory.example.yml \
        "$playbook" >/dev/null
done

bash -n installer/multipass/test/run-smoke.sh
if command -v shellcheck >/dev/null 2>&1; then
    shellcheck installer/multipass/test/run-smoke.sh
fi

real_smoke="$(printenv TUGBOAT_MULTIPASS_REAL_SMOKE || true)"
if [[ "$real_smoke" == "1" ]]; then
    cluster="$(printenv TUGBOAT_MULTIPASS_SMOKE_CLUSTER || printf '%s' tugboat-smoke)"
    config="$(printenv TUGBOAT_MULTIPASS_SMOKE_CONFIG || printf '%s' installer/multipass/config.example.json)"
    python3 installer/multipass/demo.py up "$cluster" --config "$config" --include-dirty
    python3 installer/multipass/demo.py demo "$cluster"
    python3 installer/multipass/demo.py status "$cluster" --json
    python3 installer/multipass/demo.py destroy "$cluster" --yes
fi
