import json
from pathlib import Path

import pytest

from hil import benchenv, config, flash


def test_validate_accepts_the_required_bench_knobs():
    env = {
        "DALI2RUST_DALI_QUERY_CONTENTION_RETRY": "1",
        "DALI2RUST_DALI_QUERY_CONTENT_CONFIRM": "1",
    }
    assert benchenv.validate(env) == []


def test_validate_reports_missing_and_wrong_values_separately():
    missing = benchenv.validate({"DALI2RUST_DALI_QUERY_CONTENT_CONFIRM": "1"})
    assert len(missing) == 1 and "is not set" in missing[0]

    wrong = benchenv.validate(
        {
            "DALI2RUST_DALI_QUERY_CONTENTION_RETRY": "0",
            "DALI2RUST_DALI_QUERY_CONTENT_CONFIRM": "1",
        }
    )
    assert len(wrong) == 1 and "expected" in wrong[0]


def test_bench_env_file_overrides_the_ambient_shell(tmp_path, monkeypatch):
    monkeypatch.setenv("DALI2RUST_DALI_QUERY_CONTENTION_RETRY", "0")
    (tmp_path / benchenv.BENCH_ENV_FILE).write_text(
        "# comment\n"
        "DALI2RUST_DALI_QUERY_CONTENTION_RETRY=1\n"
        'DALI2RUST_DALI_QUERY_CONTENT_CONFIRM="1"\n'
        "\n"
    )
    env = benchenv.resolve(tmp_path)
    assert benchenv.validate(env) == []


def test_shipped_example_satisfies_the_required_knobs(tmp_path):
    example = Path(__file__).resolve().parents[1] / benchenv.BENCH_ENV_EXAMPLE
    (tmp_path / benchenv.BENCH_ENV_FILE).write_text(example.read_text())
    assert benchenv.validate(benchenv.resolve(tmp_path)) == []


def test_manifest_records_provenance_and_never_secrets(tmp_path):
    repo_root = Path(__file__).resolve().parents[3]
    env = {
        "DALI2RUST_DALI_QUERY_CONTENTION_RETRY": "1",
        "DALI2RUST_DALI_QUERY_CONTENT_CONFIRM": "1",
        "DALI2RUST_PERSIST_DISABLE": "1",
        "GITHUB_TOKEN": "some-network",
        "AWS_SECRET_ACCESS_KEY": "some-password",
    }
    path = benchenv.write_manifest(
        tmp_path,
        repo_root,
        env,
        Path("target/riscv32imafc-esp-espidf/debug/dali2rust"),
        "riscv32imafc-esp-espidf",
        bench_valid=True,
    )
    raw = path.read_text()
    assert "some-network" not in raw and "some-password" not in raw

    manifest = json.loads(raw)
    assert manifest["bench_valid"] is True
    assert manifest["firmware"]["target"] == "riscv32imafc-esp-espidf"
    assert set(manifest["build_env"]) == {
        "DALI2RUST_DALI_QUERY_CONTENTION_RETRY",
        "DALI2RUST_DALI_QUERY_CONTENT_CONFIRM",
        "DALI2RUST_PERSIST_DISABLE",
    }
    assert len(manifest["git"]["commit"]) == 40
    assert isinstance(manifest["git"]["dirty"], bool)


def test_board_env_comes_from_cargos_own_config_not_a_copy():
    cargo = flash.cargo_env()
    spec = flash.board_spec(config.HilConfig(board="esp32p4"))
    assert spec["board_env"] == {
        "MCU": cargo["MCU"],
        "ESP_IDF_SDKCONFIG_DEFAULTS": cargo["ESP_IDF_SDKCONFIG_DEFAULTS"],
    }
    assert spec["board_env"]["MCU"] == "esp32p4"
    assert spec["build_env"] == {"ESP_IDF_SYS_ROOT_CRATE": "dali2rust-firmware"}
    assert "ESP_IDF_SYS_ROOT_CRATE" not in spec["board_env"]


def test_cargo_env_reads_forced_entries_as_their_value():
    cargo = flash.cargo_env()
    assert cargo["MCU"] == "esp32p4"
    assert cargo["ESP_IDF_SDKCONFIG_DEFAULTS"] == "sdkconfig.p4.defaults"
    assert cargo["ESP_IDF_SYS_ROOT_CRATE"] == "dali2rust-firmware"
    assert "fw" not in cargo and "bdd" not in cargo


def test_an_unknown_board_fails_loudly():
    with pytest.raises(flash.BoardError, match="unknown HIL_BOARD"):
        flash.board_spec(config.HilConfig(board="esp32s3"))


def test_a_repointed_cargo_env_fails_before_anything_is_flashed(monkeypatch, tmp_path):
    (tmp_path / ".cargo").mkdir()
    (tmp_path / ".cargo" / "config.toml").write_text(
        '[env]\nMCU = "esp32c6"\nESP_IDF_SDKCONFIG_DEFAULTS = "sdkconfig.c6.defaults"\n')
    monkeypatch.setattr(flash, "REPO_ROOT", tmp_path)
    with pytest.raises(flash.BoardError, match="wrong die"):
        flash.board_spec(config.HilConfig(board="esp32p4"))


def test_a_cargo_env_with_no_board_knobs_fails_loudly(monkeypatch, tmp_path):
    (tmp_path / ".cargo").mkdir()
    (tmp_path / ".cargo" / "config.toml").write_text('[env]\nESP_IDF_VERSION = "v5.5.3"\n')
    monkeypatch.setattr(flash, "REPO_ROOT", tmp_path)
    with pytest.raises(flash.BoardError, match="which architecture"):
        flash.board_spec(config.HilConfig(board="esp32p4"))
