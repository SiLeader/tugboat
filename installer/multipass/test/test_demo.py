#!/usr/bin/env python3
import json
import shutil
import stat
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SCRIPT_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPT_DIR))
import demo  # noqa: E402


class ParsingTests(unittest.TestCase):
    def test_multipass_json_shapes_are_normalized(self):
        listed = demo.parse_multipass_list(
            {"list": [{"name": "demo-cp-1", "state": "RUNNING"}]}
        )
        self.assertEqual(listed[0]["name"], "demo-cp-1")

        info = demo.parse_multipass_info(
            {"info": {"demo-cp-1": {"state": "RUNNING", "ipv4": ["192.0.2.10"]}}},
            "demo-cp-1",
        )
        self.assertEqual(info["name"], "demo-cp-1")
        self.assertEqual(demo.extract_ipv4_addresses(info), ["192.0.2.10"])

    def test_management_ip_ignores_cni_and_prefers_ssh_reachable_address(self):
        address = demo.choose_management_ip(
            ["10.244.1.3", "169.254.1.2", "192.0.2.22", "192.0.2.23"],
            "10.244.0.0/16",
            probe=lambda value: value == "192.0.2.23",
        )
        self.assertEqual(address, "192.0.2.23")

    def test_size_parser_accepts_multipass_units(self):
        self.assertEqual(demo.parse_size("6G"), 6 * 1024**3)
        self.assertEqual(demo.parse_size("512M"), 512 * 1024**2)
        with self.assertRaises(demo.DemoError):
            demo.parse_size("0G")


class StateAndInventoryTests(unittest.TestCase):
    def test_playbook_commands_resolve_existing_entrypoints(self):
        with tempfile.TemporaryDirectory() as temporary:
            with mock.patch.object(demo, "cache_root", return_value=Path(temporary)):
                state = demo.new_state("demo", json.loads(json.dumps(demo.DEFAULT_CONFIG)))
                for index, node in enumerate(state["nodes"], start=10):
                    node["ip"] = f"192.0.2.{index}"
                for name in ("prepare.yml", "site.yml", "verify.yml", "demo.yml", "collect.yml"):
                    with self.subTest(playbook=name):
                        with mock.patch.object(demo, "run_command") as run:
                            run.return_value = demo.subprocess.CompletedProcess([], 0, "", "")
                            demo.run_playbook(state, name)
                        args, kwargs = run.call_args
                        path = Path(args[0][-1])
                        self.assertTrue(path.is_file(), path)
                        expected = demo.ANSIBLE_ROOT if name == "site.yml" else SCRIPT_DIR / "ansible"
                        self.assertEqual(path.parent, expected)
                        self.assertEqual(kwargs["cwd"], demo.ANSIBLE_ROOT)

    def test_cluster_option_and_positional_forms_are_available(self):
        parser = demo.make_parser()
        option_args = parser.parse_args(["up", "--cluster", "trial"])
        positional_args = parser.parse_args(["up", "trial"])
        self.assertEqual(option_args.cluster_option, "trial")
        self.assertEqual(positional_args.cluster_positional, "trial")

    def test_state_and_inventory_are_cluster_scoped(self):
        with tempfile.TemporaryDirectory() as temporary:
            with mock.patch.object(demo, "cache_root", return_value=Path(temporary)):
                config = json.loads(json.dumps(demo.DEFAULT_CONFIG))
                state = demo.new_state("demo", config)
                directory = demo.cluster_dir("demo")
                directory.mkdir(mode=0o700)
                demo.save_state(directory, state)
                state = demo.load_state(directory)
                state["nodes"][0]["ip"] = "192.0.2.10"
                state["nodes"][1]["ip"] = "192.0.2.11"
                state["nodes"][2]["ip"] = "192.0.2.12"
                inventory, variables = demo.build_inventory(state)

            self.assertEqual(
                inventory["all"]["children"]["tugboat_workers"]["hosts"],
                {"demo-worker-1": {}, "demo-worker-2": {}},
            )
            self.assertEqual(
                inventory["all"]["children"]["tugboat_control_plane"]["hosts"],
                {"demo-cp-1": {}},
            )
            self.assertEqual(
                inventory["all"]["children"]["tugboat_csi_hostpath"]["hosts"],
                {},
            )
            self.assertEqual(
                inventory["all"]["vars"]["tugboat_apiserver_advertise_url"],
                "https://192.0.2.10:8443",
            )
            self.assertEqual(variables["tugboat_multipass_expected_workers"], 2)
            self.assertTrue(inventory["all"]["vars"]["tugboat_csi_hostpath_enabled"] is False)

    def test_cloud_init_contains_owner_and_uses_private_permissions(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "cloud-init.yaml"
            demo.render_cloud_init(
                output,
                ssh_public_key="ssh-ed25519 AAAA test",
                cluster_uuid="11111111-1111-1111-1111-111111111111",
                node_name="demo-worker-1",
                node_role="worker",
            )
            content = output.read_text(encoding="utf-8")
            self.assertIn("11111111-1111-1111-1111-111111111111", content)
            self.assertIn("demo-worker-1", content)
            self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o600)


class SafetyTests(unittest.TestCase):
    def test_owner_check_can_keep_an_owned_stopped_vm_running(self):
        state = demo.new_state("demo", json.loads(json.dumps(demo.DEFAULT_CONFIG)))
        name = state["nodes"][0]["name"]
        mp = mock.Mock()
        mp.list.return_value = [{"name": name, "state": "STOPPED"}]
        mp.owner.return_value = state["cluster_uuid"]

        with mock.patch.object(demo, "wait_for_node"):
            demo.verify_owner(mp, state, name, restore_stopped=False)

        mp.start.assert_called_once()
        mp.stop.assert_not_called()

    def test_owner_check_restores_an_unowned_stopped_vm(self):
        state = demo.new_state("demo", json.loads(json.dumps(demo.DEFAULT_CONFIG)))
        name = state["nodes"][0]["name"]
        mp = mock.Mock()
        mp.list.return_value = [{"name": name, "state": "STOPPED"}]
        mp.owner.return_value = "another-cluster"

        with (
            mock.patch.object(demo, "wait_for_node"),
            self.assertRaisesRegex(demo.DemoError, "owner UUID"),
        ):
            demo.verify_owner(mp, state, name, restore_stopped=False)

        mp.start.assert_called_once()
        mp.stop.assert_called_once()

    def test_status_reports_stopped_nodes_as_not_run(self):
        with tempfile.TemporaryDirectory() as temporary:
            with mock.patch.object(demo, "cache_root", return_value=Path(temporary)):
                state = demo.new_state("demo", json.loads(json.dumps(demo.DEFAULT_CONFIG)))
                directory = demo.cluster_dir("demo")
                directory.mkdir(mode=0o700)
                demo.save_state(directory, state)
                listed = [
                    {"name": node["name"], "state": "STOPPED"}
                    for node in state["nodes"]
                ]
                args = mock.Mock(cluster="demo", json=True)
                with (
                    mock.patch.object(demo.Multipass, "list", return_value=listed),
                    mock.patch("builtins.print") as print_output,
                ):
                    result = demo.run_status(args)

        self.assertEqual(result, 0)
        output = json.loads(print_output.call_args.args[0])
        self.assertEqual(output["verification"], "not-run")

    def test_guest_probe_is_disabled_until_image_credentials_are_configured(self):
        config = json.loads((SCRIPT_DIR / "config.example.json").read_text(encoding="utf-8"))
        self.assertFalse(config["demo"]["guest_probe"]["enabled"])
        demo.validate_config(config)

    def test_enabled_guest_probe_requires_local_identity_and_guest_host_key(self):
        with tempfile.TemporaryDirectory() as temporary:
            identity = Path(temporary) / "id_ed25519"
            identity.write_text("private key", encoding="utf-8")
            config = json.loads(json.dumps(demo.DEFAULT_CONFIG))
            probe = config["demo"]["guest_probe"]
            probe["enabled"] = True
            probe["ssh_identity_file"] = str(identity)
            probe["ssh_host_public_key"] = "ssh-ed25519 AAAA test"
            demo.validate_config(config)

            for key in ("ssh_identity_file", "ssh_host_public_key"):
                with self.subTest(invalid=key):
                    invalid = json.loads(json.dumps(config))
                    invalid["demo"]["guest_probe"][key] = ""
                    with self.assertRaisesRegex(demo.DemoError, key):
                        demo.validate_config(invalid)

    def test_ip_change_failure_marks_cluster_for_recreation(self):
        with tempfile.TemporaryDirectory() as temporary:
            with mock.patch.object(demo, "cache_root", return_value=Path(temporary)):
                state = demo.new_state("demo", json.loads(json.dumps(demo.DEFAULT_CONFIG)))
                directory = demo.cluster_dir("demo")
                directory.mkdir(mode=0o700)
                state["stage"] = "waiting"
                state["nodes"][0]["state"] = "needs-recreate"
                demo.record_operation_failure(
                    directory,
                    state,
                    demo.DemoError("management IP changed"),
                )
                saved = demo.load_state(directory)

        self.assertEqual(saved["stage"], "needs-recreate")
        self.assertEqual(saved["error"], "management IP changed")

    def test_demo_refuses_to_skip_guest_connectivity_probe(self):
        with tempfile.TemporaryDirectory() as temporary:
            with mock.patch.object(demo, "cache_root", return_value=Path(temporary)):
                config = json.loads(json.dumps(demo.DEFAULT_CONFIG))
                config["demo"]["guest_probe"]["enabled"] = False
                state = demo.new_state("demo", config)
                directory = demo.cluster_dir("demo")
                directory.mkdir(mode=0o700)
                demo.save_state(directory, state)
                args = mock.Mock(cluster="demo", cleanup=False)
                with self.assertRaisesRegex(demo.DemoError, "guest_probe.enabled=true"):
                    demo.run_demo(args)

    def test_collect_restores_started_nodes_after_owner_check_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            with mock.patch.object(demo, "cache_root", return_value=Path(temporary)):
                state = demo.new_state("demo", json.loads(json.dumps(demo.DEFAULT_CONFIG)))
                directory = demo.cluster_dir("demo")
                directory.mkdir(mode=0o700)
                demo.save_state(directory, state)
                listed = [
                    {"name": node["name"], "state": "STOPPED"}
                    for node in state["nodes"]
                ]
                mp = mock.Mock()
                mp.list.return_value = listed
                args = mock.Mock(cluster="demo")
                with (
                    mock.patch.object(demo, "Multipass", return_value=mp),
                    mock.patch.object(
                        demo,
                        "verify_owner",
                        side_effect=[True, demo.DemoError("owner mismatch")],
                    ),
                    self.assertRaisesRegex(demo.DemoError, "owner mismatch"),
                ):
                    demo.run_collect(args)

        mp.stop.assert_called_once_with(
            [state["nodes"][0]["name"]],
            timeout=float(state["config"]["multipass"]["launch_timeout_seconds"]),
        )

    def test_external_manifest_requires_all_release_binaries(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            for name in demo.REQUIRED_BINARIES:
                path = directory / name
                shutil.copy2("/bin/true", path)
                path.chmod(0o755)
            manifest, digest = demo.external_binary_manifest(directory)
            self.assertEqual(manifest["mode"], "external-prebuilt")
            self.assertEqual(len(manifest["binaries"]), len(demo.REQUIRED_BINARIES))
            self.assertEqual(len(digest), 64)

    def test_external_manifest_rejects_truncated_elf(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            for name in demo.REQUIRED_BINARIES:
                path = directory / name
                header = bytearray(20)
                header[:4] = b"\x7fELF"
                header[4] = 2
                header[5] = 1
                header[18:20] = (62).to_bytes(2, "little")
                path.write_bytes(header)
                path.chmod(0o755)
            with self.assertRaisesRegex(demo.DemoError, "not a Linux x86_64 ELF"):
                demo.external_binary_manifest(directory)

    def test_external_manifest_rejects_empty_elf_interpreter(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            for name in demo.REQUIRED_BINARIES:
                path = directory / name
                shutil.copy2("/bin/true", path)
                path.chmod(0o755)
            path = directory / demo.REQUIRED_BINARIES[0]
            contents = bytearray(path.read_bytes())
            program_header_offset = int.from_bytes(contents[32:40], "little")
            program_header_size = int.from_bytes(contents[54:56], "little")
            program_header_count = int.from_bytes(contents[56:58], "little")
            for index in range(program_header_count):
                offset = program_header_offset + index * program_header_size
                if int.from_bytes(contents[offset : offset + 4], "little") == 3:
                    contents[offset + 32 : offset + 40] = (0).to_bytes(8, "little")
                    break
            else:
                self.fail("/bin/true has no PT_INTERP segment")
            path.write_bytes(contents)
            with self.assertRaisesRegex(demo.DemoError, "invalid ELF interpreter"):
                demo.external_binary_manifest(directory)

    def test_external_manifest_runs_readelf_in_c_locale(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            for name in demo.REQUIRED_BINARIES:
                path = directory / name
                shutil.copy2("/bin/true", path)
                path.chmod(0o755)
            readelf_result = demo.subprocess.CompletedProcess(
                [], 0, "There is no dynamic section in this file.", ""
            )
            with (
                mock.patch.object(
                    demo.shutil,
                    "which",
                    side_effect=lambda name: "/usr/bin/readelf" if name == "readelf" else None,
                ),
                mock.patch.object(demo, "run_command", return_value=readelf_result) as run,
            ):
                demo.external_binary_manifest(directory)
            for call in run.call_args_list:
                self.assertEqual(call.kwargs["env"]["LC_ALL"], "C")
                self.assertEqual(call.kwargs["env"]["LANG"], "C")

    def test_private_json_write_is_atomic_and_mode_0600(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "state.json"
            demo.write_json_atomic(path, {"secret": "value"})
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            self.assertEqual(json.loads(path.read_text(encoding="utf-8"))["secret"], "value")

    def test_state_rejects_paths_outside_the_cluster_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            with mock.patch.object(demo, "cache_root", return_value=Path(temporary)):
                directory = demo.cluster_dir("demo")
                directory.mkdir(mode=0o700)
                state = demo.new_state("demo", json.loads(json.dumps(demo.DEFAULT_CONFIG)))
                state["paths"]["vars"] = "/tmp/outside-vars.json"
                demo.write_json_atomic(directory / "state.json", state)
                with self.assertRaises(demo.DemoError):
                    demo.load_state(directory)

    def test_destroy_uses_only_state_names(self):
        self.assertTrue(callable(demo.destroy_local_secrets))
        self.assertNotIn("--all", Path(demo.__file__).read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
