# CP2K functional test inputs for e2e-images (CPU variants)

Hard-gate functional tests for the CI-built CPU images:

- **H2O suite** -- image `cp2k/cp2k:2026.2_mpich_generic_psmp` (variant
  `toolchain_2026.2_generic`). Four pinned upstream H2O inputs are run for
  real in the container (`mpirun -np 2 cp2k -i <inp> -o <inp>.out`) and their
  output is asserted by `assert_cp2k_output.sh`. This goes one step past the
  `cp2k --version` smoke test: it proves the image can actually resolve basis
  sets, potentials and DFTB parameters and converge real CP2K runs.
- **deepmd water test** -- image `cp2k/cp2k:2026.2_mpich_generic_psmp_deepmd`
  (variant `toolchain_2026.2_generic_deepmd`, built with `--features deepmd`).
  The upstream regtest-deepmd input is run at `-np 1` and `-np 2` and its
  energy is asserted against the upstream reference via the pinned value in
  `assert_cp2k_output.sh`. This goes past the `cp2kflags` token gate: it
  proves the DeePMD/libtorch build actually computes (and that the MPI-shared
  DeePMD force path is rank-invariant).

Wired into `.github/workflows/e2e-images.yml`: step
`functional test: H2O suite (2026.2 CPU variant; hard gate)` gated on
`inputs.variant == 'toolchain_2026.2_generic'`, and steps
`smoke + cp2kflags hard gate (2026.2 deepmd variant)` plus
`functional test: deepmd water (2026.2 deepmd variant; hard gate)` gated on
`inputs.variant == 'toolchain_2026.2_generic_deepmd'` (GPU variants are
deliberately not covered: the runner has no NVIDIA driver).

## Source table

All five files are byte-exact copies from upstream pin `support/v2026.2`
(fetched 2026-10-15 via `gh api repos/cp2k/cp2k/contents/<path>?ref=support/v2026.2`).
Byte-exactness is recorded two ways: the local `git hash-object` blob SHA
equals the upstream blob SHA returned by the GitHub API, and the sha256 below
is the checksum of the checked-in copy.

| local file | upstream path @ `support/v2026.2` | git blob SHA | sha256 | purpose |
|---|---|---|---|---|
| `h2o-quickstep.inp` | `docs/getting-started/h2o.inp` | `c401d5ece3ea2ce8300cb4254a4129d5cbc4c6fa` | `ebd639b3ebed2311182f92b1362dec4457f733149eeabb37686663d87cfd7fde` | Quickstep/QS getting-started energy run (PBE, MOLOPT basis) |
| `h2o-qs-ot.inp` | `tests/QS/regtest-ot-2/H2O-1.inp` | `d073c2810ca53c0da5a5b03d0a5af153e8172fa1` | `a81798949c20ab0e46e0d26d15c82b454b5e0c0ebb3e36e7ba81f4873d1c714d` | QS orbital-transformation (OT) SCF path, PADE functional |
| `h2o-se.inp` | `tests/SE/regtest/h2o.inp` | `5606661de141007aa1706fdaf685bcd6be500720` | `ccd969e7d0d2109b2c478d97015aac023d909e7dd3e76bf46b2d3053cdb87414` | semi-empirical (AM1) geometry optimization |
| `h2o-dftb-scc.inp` | `tests/DFTB/regtest-scc/h2o-1.inp` | `d255fd7410f54c0145e9a521d3b3eda0357ec9bb` | `5be862b8c996efc390e6324bc3c0ca7116d171cdf18df8aca08f95267dfd00d6` | DFTB SCC NPT molecular dynamics (64-water periodic box) |
| `deepmd-water.inp` | `tests/Fist/regtest-deepmd/deepmd_water.inp` | `b2638fe2832659b4662cfcf22f81b31e199eb71f` | `7c472e6165bdc743e57ae5546a1d7b2c8cf779a919e3b264264bc8cc3d5eb9fc` | DeePMD (PyTorch `.pth` model) FIST energy/force run: `RUN_TYPE ENERGY_FORCE` + `METHOD FIST` + `&MM/&FORCEFIELD/&NONBONDED/&DEEPMD` |

Path note (H2O/DFTB entry): the task brief cited
`tests/DFTB/regtest-scc-1-1/h2o-1.inp`; that path does not exist at the pin
(the DFTB tree has `regtest-scc`, `regtest-scc-2`, `regtest-scc-spglib`,
`regtest-nonscc`, ... but no `regtest-scc-1-1`). The closest real path
`tests/DFTB/regtest-scc/h2o-1.inp` is used and is itself the canonical SCC
`h2o-1` test.

## Self-containment (verified by reading each file in full)

No input contains an active `&INCLUDE` or references any file outside the
image's `/opt/cp2k/data/` tree. Exact per-input findings:

- `h2o-quickstep.inp`: no include directives. Resolves `BASIS_MOLOPT` and
  `GTH_POTENTIALS` (plain names, no path) through `CP2K_DATA_DIR`.
- `h2o-qs-ot.inp`: no include directives. Resolves `BASIS_SET` and
  `POTENTIAL` through `CP2K_DATA_DIR`.
- `h2o-se.inp`: no external files at all (AM1 has no basis/potential files).
- `h2o-dftb-scc.inp`: the five `#CPQA INCLUDE ...` lines are comments (test
  harness metadata), not active CP2K includes. At runtime it needs
  `PARAM_FILE_PATH DFTB/scc` + `PARAM_FILE_NAME scc_parameter` and
  `UFF_FORCE_FIELD ../uff_table`, i.e. `data/DFTB/scc/{scc_parameter,oo.spl,
  oh.spl,ho.spl,hh.spl}` and `data/DFTB/uff_table` (all present in the image
  at `/opt/cp2k/data/`, cross-checked against the upstream tree at the same
  pin). These relative paths resolve against `CP2K_DATA_DIR`.
- `deepmd-water.inp`: no include directives. Its only external dependency is
  the model file `POT_FILE_NAME DeePMD/water_dpa2.pth` (relative path). CP2K
  resolves it with `discover_file()`: cwd first, then the compiled-in data
  dir. The model ships inside the image at
  `/opt/cp2k/data/DeePMD/water_dpa2.pth` (226,164 B; upstream blob
  `e2b26abf9f83b9d13dda9375dddb89b67a576fb7` at the same pin), so **no extra
  mount or download is needed** -- the workflow mounts only `ci/cp2k-tests`.
  (The model is a PyTorch `.pth` frozen model, matching the toolchain's
  `-DENABLE_PYTORCH=TRUE` libdeepmd_c build; upstream's `deepmd.md` still
  mentions `.pb` in prose, the code path and the regtest use `.pth`.)

**Data dir: compiled in, no environment variable needed.** The release-cmake
arm of the generator pins `-DCP2K_DATA_DIR=/opt/cp2k/data`
(`src/build/generate.rs`, wave ci-h2o; upstream variable is the cache entry
`CP2K_DATA_DIR`, consumed as the `__DATA_DIR` macro in
`src/CMakeLists.txt` and read by `src/common/cp_data_dir.c`). Without the pin,
upstream's default is the sentinel `"default"`, which `src/CMakeLists.txt`
resolves at configure time to `${CMAKE_INSTALL_FULL_DATAROOTDIR}/cp2k/data`
and installs the data tree to — an install-relative share path, not the
`/opt/cp2k/data` location this image itself ships and documents. With the pin
the binary resolves basis sets, potentials, DFTB parameters and the DeePMD
model straight from `/opt/cp2k/data`, so the workflow passes **no**
`CP2K_DATA_DIR` and the runs validate the image's out-of-the-box behavior.
(Upstream's own `tests/do_regtest.py` sets the env var because it runs against
a source checkout, not an installed image.)

## Expected values

`assert_cp2k_output.sh` checks the last `ENERGY| Total FORCE_EVAL ( ... )
energy [a.u.]:` line of each output (CP2K v2026.2 prints this line in the same
format for every force-evaluation method; QS/DFT, SE and DFTB all run inside
the QS framework, so their label is `( QS )`; the deepmd run is `METHOD FIST`,
so its label is `( FIST )` and the same pattern covers it).

Pinned label (exact value +- tolerance):

| label (input stem) | expected (a.u.) | tolerance | source |
|---|---|---|---|
| `deepmd-water` | `-17.266752586704772` | `1e-6` | upstream `tests/Fist/regtest-deepmd/TEST_FILES.toml` @ `support/v2026.2` (blob `248c45a29efd0055f7a9903132e6900e27ee3c97`): `"deepmd_water.inp" = [{matcher="M011", tol=1.0E-6, ref=-17.266752586704772}]`, with `M011 = GenericMatcher(r"ENERGY| Total FORCE_EVAL", col=9)` (`tests/matchers.py`); pinned identically in `assert_cp2k_output.sh` (`PINNED[deepmd-water]`) |

H2O suite labels: **sanity mode -- exact values to be pinned after a CI
baseline run.** Intervals:

| label (input stem) | sanity interval (a.u.) | rationale |
|---|---|---|
| `h2o-quickstep` | `[-100, -5]` | single H2O cell, QS/DFT total energy |
| `h2o-qs-ot` | `[-100, -5]` | single H2O cell, QS/OT total energy |
| `h2o-se` | `[-100, -5]` | single H2O cell; the printed a.u. energy is the eV `Total energy [eV]:` value / 27.2114 |
| `h2o-dftb-scc` | `[-1000, -5]` | 64-water periodic box: SCC total energy is ~-130.6 a.u. (upstream `tests/DFTB/regtest-scc/TEST_FILES.toml` reference for `h2o-1.inp` is -130.55701170331295), i.e. outside the generic `[-100, -5]` bracket; this is interval evidence only, not a pin |

Pinning: set `PINNED[<label>]="<expected>:<tolerance>"` in
`ci/cp2k-tests/assert_cp2k_output.sh`. Pinned labels get a strict
`|value - expected| <= tolerance` check; unpinned labels print
`UNPINNED <label> (sanity mode)` and only take the interval check. For the H2O
suite the baseline values can be read off the `FUNCTIONAL evidence: ENERGY
lines` block the workflow step prints at its end; `deepmd-water` is pinned
from the start because the upstream regtest reference makes a baseline run
unnecessary.

## Failure triage

- `ASSERT FAIL ... exit code is not 0`: the CP2K run died. Read the
  `FUNCTIONAL run begin/end` block above it; typical causes:
  - `... cannot open file ...` / missing basis set -> the compiled-in data
    dir regressed (the `-DCP2K_DATA_DIR=/opt/cp2k/data` pin in
    `src/build/generate.rs`; guarded by the `t_ci_h2o` tests) — the
    workflow intentionally passes no `CP2K_DATA_DIR`;
  - MPI errors at startup -> `--shm-size=1g` is mandatory for mpich inside
    docker (upstream test containers require the same);
  - DFTB: `scc_parameter` / `*.spl` not found -> the `DFTB/scc` resolution
    failed, same compiled-in data-dir cause.
- `missing 'PROGRAM ENDED AT'`: the run was killed (OOM/timeout) or aborted
  mid-way after a nonzero soft exit.
- `energy value is not numeric`: the ENERGY line carries `NaN`/`Inf` or a
  Fortran overflow marker.
- `pinned value out of tolerance` (`deepmd-water`): the DeePMD computation
  drifted from the upstream reference. Separate the two obvious causes first:
  a CP2K/DeePMD version or model change (the upstream reference is tied to the
  pin), versus a genuinely broken build. The `-np 1` and `-np 2` runs are
  asserted separately, so a rank-dependent mismatch (MPI sharing bug) shows up
  as the `np2` iteration failing while `np1` passes.
- `deepmd-water` run dies with `water_dpa2.pth` not found -> the data-dir
  resolution failed for the model path (`/opt/cp2k/data/DeePMD/`), same
  compiled-in data-dir cause as above.
- `energy value out of sanity interval`: either a real regression or the
  interval for that label needs re-derivation (see the table above).
- `UNPINNED ... (sanity mode)`: informational, not a failure.
- Assertion script usage is `assert_cp2k_output.sh <output_file>
  <test_label> <exit_code>`; the label must match the table/`PINNED` keys
  (input stem without `.inp`).

## Local rehearsal

The script is deliberately pure bash/awk/grep so the red/green paths can be
exercised locally on Git Bash without docker (see the results transcript in
`.omz/runtime/.../results/h2o-functional.md` for a recorded run of all five
dual-state fixtures; the deepmd-water pinned drill -- within tolerance =
green, beyond tolerance = red, missing ENERGY line = red -- is recorded in
`.omz/runtime/.../results/preset-ci.md`).

The deepmd functional step can also be reproduced against a locally built
image (the model file is already inside it, so only the inputs are mounted):

```sh
docker run --rm --shm-size=1g -v "$PWD/ci/cp2k-tests:/work" -w /work \
  cp2k/cp2k:2026.2_mpich_generic_psmp_deepmd \
  bash -c "mpirun -np 2 cp2k -i deepmd-water.inp -o deepmd-water.np2.out"
bash ci/cp2k-tests/assert_cp2k_output.sh \
  ci/cp2k-tests/deepmd-water.np2.out deepmd-water 0
```
