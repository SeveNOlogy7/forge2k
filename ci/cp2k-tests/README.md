# CP2K functional test inputs for e2e-images (CPU variants)

Functional tests for the CI-built CPU images (the H2O suite is a hard gate;
the deepmd water test is a hard gate again after the OpenBLAS
SMALL_MATRIX_OPT fix, see below):

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
  **Status (updated 2026-10-21): FIXED AND VERIFIED -- hard gate restored.**
  With OpenBLAS built `SMALL_MATRIX_OPT=0` (fix commit f58aeed) the
  computation-level run passes: e2e run 39286488384 has `deepmd-water`
  np1/np2 rc=0 and the energy -17.266752586704772 is bit-identical to the
  upstream TEST_FILES.toml M011 reference. The pin below is retained and the
  CI step blocks the run on failure again; the original SIGSEGV
  investigation is kept as history in the section below. Build step and
  cp2kflags gate pass.

Wired into `.github/workflows/e2e-images.yml`: step
`functional test: H2O suite (2026.2 CPU variant; hard gate)` gated on
`inputs.variant == 'toolchain_2026.2_generic'`, and steps
`smoke + cp2kflags hard gate (2026.2 deepmd variant)` (hard gate) plus
`functional test: deepmd water (2026.2 deepmd variant; hard gate)` gated on
`inputs.variant == 'toolchain_2026.2_generic_deepmd'` (GPU variants are
deliberately not covered: the runner has no NVIDIA driver).

## deepmd-water: known upstream issue (status 2026-10-16) -- RESOLVED 2026-10-21

**Resolution:** the crash was root-caused (core dump + gdb + source-level
evidence, runs 39037606858 / 39045939348) to the toolchain-built **OpenBLAS
0.3.33 SMALL_MATRIX_OPT** small-matrix `gemm_batch` dispatch defect, not to
deepmd-kit/libtorch (upstream fix PR #5980 is develop-only; v0.3.33/v0.3.34
ship without it). Fix: the `deepmd` preset injects a RUN before the toolchain
install that builds OpenBLAS with `SMALL_MATRIX_OPT=0` (commit f58aeed). The
defect no longer reproduces: e2e run 39286488384 has `deepmd-water` np1/np2
rc=0 and the energy -17.266752586704772 is bit-identical to the upstream
TEST_FILES.toml M011 reference. The CI step is a hard gate again
(`continue-on-error` removed). The paragraphs below are the historical record
of the investigation while it was failing (kept for the evidence chain).

The `deepmd-water` computation-level test used to **fail**, and its CI step
was **informational** (`continue-on-error: true`) at the time: it did not
block the e2e run, but it still executed with the full assertion. The pinned
reference and the byte-exact input are retained -- they now guard the fixed
build as a hard gate.

Observed failure (identical in both e2e rounds): `mpirun -np 1` / `-np 2 cp2k
-i deepmd-water.inp` -> rc=139 (SIGSEGV) at compute time, after the DeePMD
model loads successfully; np1/np2 die at the same point. The build step and
the cp2kflags gate (deepmd + libtorch tokens) pass in both rounds.

Evidence chain (runs):

| run | what it established |
|---|---|
| 38682037203 | round 1: build + cp2kflags PASS; `deepmd-water` rc=139 SIGSEGV, np1/np2 same point |
| 38695004330 | wave-t4-fix probe: `ldd cp2k.psmp` listed BOTH the system `/lib/x86_64-linux-gnu/libgomp.so.1` and torch's `libgomp-98b21ff3.so.1`; single-thread/scoped env inert, rc=139 persists (E3); CXX11 ABI already ruled out (`__cxx11` symbol count consistent) |
| 38707255454 | round 2, with the libgomp-unify fix in the image: crash point byte-identical to round 1 -> the duplicate-libgomp hypothesis (E2) is refuted; the empirically ineffective unify fix is reverted in git (attempt kept in history) |

Root-cause candidates after the two-round experiment matrix (E1 gdb no
output / E2 dual libgomp found -> fix -> refuted by round 2 / E3 thread env
inert): the upstream **deepmd-kit 3.1.0 + libtorch 2.7.1 cxx11-abi**
combination itself breaking at runtime in an environment upstream does not
validate. Side evidence that upstream does not verify this combination:
cp2k's own toolchain image builds with `DEEPMD=OFF`, and the spack route pins
`deepmdkit@3.1.3`. Filing an upstream issue is a backlog item pending user
authorization.

Decision record / full transcript: maintainer-local
`.omz/runtime/<stem>/results/deepmd-degrade.md` (plus the refuted-fix
record `deepmd-gomp-fix.md`, the root-cause report `deepmd-root-cause.md`
and the fix transcript `deepmd-fix.md` in the same directory); note `.omz/` is
gitignored and therefore not reachable from the repository — this
README section is the in-repo index.

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

Pinned labels (exact value +- tolerance):

| label (input stem) | expected (a.u.) | tolerance | source |
|---|---|---|---|
| `h2o-quickstep` | `-17.212337144585160` | `1e-6` | CI baseline run `38676712292` (2026-10-15, success; variant `toolchain_2026.2_generic`, image `cp2k/cp2k:2026.2_mpich_generic_psmp`), last `ENERGY|` line |
| `h2o-qs-ot` | `-15.642164448397811` | `1e-6` | CI baseline run `38676712292`, last `ENERGY|` line |
| `h2o-se` | `-12.809418844339213` | `1e-6` | CI baseline run `38676712292`, last `ENERGY|` line |
| `h2o-dftb-scc` | `-130.557012120984041` | `1e-6` | CI baseline run `38676712292`, last `ENERGY|` line |
| `deepmd-water` | `-17.266752586704772` | `1e-6` | upstream `tests/Fist/regtest-deepmd/TEST_FILES.toml` @ `support/v2026.2` (blob `248c45a29efd0055f7a9903132e6900e27ee3c97`): `"deepmd_water.inp" = [{matcher="M011", tol=1.0E-6, ref=-17.266752586704772}]`, with `M011 = GenericMatcher(r"ENERGY| Total FORCE_EVAL", col=9)` (`tests/matchers.py`) |

Tolerance semantics: the pinned number is what the same image with the
byte-exact inputs reproduces (baseline run `38676712292`); 1e-6 a.u. absorbs
cross-machine floating-point and MPI-reduction-order jitter between runs
while still catching any real regression (version/model/physics change).
The pin is applied to the LAST `ENERGY| Total FORCE_EVAL ( ... ) energy
[hartree]:` line of the output — the same line the baseline values were read
from and the same one `assert_cp2k_output.sh` extracts (`tail -n 1`). (The
unit token printed by this build is `[hartree]`, the atomic unit of energy;
the script's pattern deliberately does not pin the unit token, so an upstream
unit-string rename cannot break the gate. Multi-step runs print one ENERGY
line per force evaluation — e.g. `h2o-se` printed 7, `h2o-dftb-scc` 3 — and
only the last one is asserted.)

Pinning mechanics: set `PINNED[<label>]="<expected>:<tolerance>"` in
`ci/cp2k-tests/assert_cp2k_output.sh`. Pinned labels get a strict
`|value - expected| <= tolerance` check; unpinned labels print
`UNPINNED <label> (sanity mode)` and take only the per-label sanity interval
below. All five labels above are pinned; the intervals stay in the script as
documentation/fallback for any label without a pin:

| label (input stem) | sanity interval fallback (a.u.) | rationale |
|---|---|---|
| `h2o-quickstep` | `[-100, -5]` | single H2O cell, QS/DFT total energy |
| `h2o-qs-ot` | `[-100, -5]` | single H2O cell, QS/OT total energy |
| `h2o-se` | `[-100, -5]` | single H2O cell; the printed a.u. energy is the eV `Total energy [eV]:` value / 27.2114 |
| `h2o-dftb-scc` | `[-1000, -5]` | 64-water periodic box: SCC total energy is ~-130.6 a.u., outside the generic `[-100, -5]` bracket |

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
- `pinned value out of tolerance` (any pinned label): the computation
  drifted from its reference. For the H2O suite labels the reference is
  baseline run `38676712292` (2026-10-15); with unchanged byte-exact inputs
  and image a drift beyond 1e-6 a.u. indicates a regression, while a
  deliberate toolchain/CP2K rebuild that shifts physics legitimately requires
  re-baselining from a fresh green run. For `deepmd-water`: the DeePMD
  computation drifted from the upstream reference. Separate the two obvious
  causes first: a CP2K/DeePMD version or model change (the upstream reference
  is tied to the pin), versus a genuinely broken build. The `-np 1` and
  `-np 2` runs are asserted separately, so a rank-dependent mismatch (MPI
  sharing bug) shows up as the `np2` iteration failing while `np1` passes.
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
