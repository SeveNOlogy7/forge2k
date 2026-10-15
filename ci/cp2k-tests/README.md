# CP2K H2O functional suite (e2e-images CPU variant)

Hard-gate functional test for the CI-built CPU image
`cp2k/cp2k:2026.2_mpich_generic_psmp`. Four pinned upstream H2O inputs are run
for real in the container (`mpirun -np 2 cp2k -i <inp> -o <inp>.out`) and their
output is asserted by `assert_cp2k_output.sh`. This goes one step past the
`cp2k --version` smoke test: it proves the image can actually resolve basis
sets, potentials and DFTB parameters and converge real CP2K runs.

Wired into `.github/workflows/e2e-images.yml`, step
`functional test: H2O suite (2026.2 CPU variant; hard gate)`, gated on
`inputs.variant == 'toolchain_2026.2_generic'` (GPU variants are deliberately
not covered: the runner has no NVIDIA driver).

## Source table

All four files are byte-exact copies from upstream pin `support/v2026.2`
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

Path note: the task brief cited `tests/DFTB/regtest-scc-1-1/h2o-1.inp`; that
path does not exist at the pin (the DFTB tree has `regtest-scc`, `regtest-scc-2`,
`regtest-scc-spglib`, `regtest-nonscc`, ... but no `regtest-scc-1-1`). The
closest real path `tests/DFTB/regtest-scc/h2o-1.inp` is used and is itself the
canonical SCC `h2o-1` test.

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

**`CP2K_DATA_DIR` requirement.** The cmake build compiles the data-dir
default to the literal string `default` (upstream `CMakeLists.txt`:
`set(CP2K_DATA_DIR "default" CACHE STRING ...)`, consumed as
`__DATA_DIR` in `src/CMakeLists.txt`; `src/common/cp_data_dir.c` returns it
when the env var is unset). The image itself sets no `CP2K_DATA_DIR`
(entrypoint.sh only sources the toolchain setup and sets `OMP_STACKSIZE`), so
every `docker run` in the workflow exports `-e CP2K_DATA_DIR=/opt/cp2k/data`
-- exactly what upstream's own harness does (`tests/do_regtest.py` always
exports `CP2K_DATA_DIR` before launching `cp2k`). Without it the runs abort
on the first missing basis-set file.

## Expected values

Status: **sanity mode -- exact values to be pinned after a CI baseline run.**

`assert_cp2k_output.sh` checks the last `ENERGY| Total FORCE_EVAL ( ... )
energy [a.u.]:` line of each output (CP2K v2026.2 prints this line in the same
format for every force-evaluation method; QS/DFT, SE and DFTB all run inside
the QS framework, so the label is always `( QS )`). Intervals:

| label (input stem) | sanity interval (a.u.) | rationale |
|---|---|---|
| `h2o-quickstep` | `[-100, -5]` | single H2O cell, QS/DFT total energy |
| `h2o-qs-ot` | `[-100, -5]` | single H2O cell, QS/OT total energy |
| `h2o-se` | `[-100, -5]` | single H2O cell; the printed a.u. energy is the eV `Total energy [eV]:` value / 27.2114 |
| `h2o-dftb-scc` | `[-1000, -5]` | 64-water periodic box: SCC total energy is ~-130.6 a.u. (upstream `tests/DFTB/regtest-scc/TEST_FILES.toml` reference for `h2o-1.inp` is -130.55701170331295), i.e. outside the generic `[-100, -5]` bracket; this is interval evidence only, not a pin |

Pinning (later): set `PINNED[<label>]="<expected>:<tolerance>"` in
`ci/cp2k-tests/assert_cp2k_output.sh` from the numbers a green CI baseline run
prints. Pinned labels get a strict `|value - expected| <= tolerance` check;
unpinned labels print `UNPINNED <label> (sanity mode)` and only take the
interval check. The baseline values can be read off the `FUNCTIONAL evidence:
ENERGY lines` block the workflow step prints at its end.

## Failure triage

- `ASSERT FAIL ... exit code is not 0`: the CP2K run died. Read the
  `FUNCTIONAL run begin/end` block above it; typical causes:
  - `... cannot open file ...` / missing basis set -> `CP2K_DATA_DIR` was not
    passed (see above);
  - MPI errors at startup -> `--shm-size=1g` is mandatory for mpich inside
    docker (upstream test containers require the same);
  - DFTB: `scc_parameter` / `*.spl` not found -> the `DFTB/scc` resolution
    failed, same data-dir cause.
- `missing 'PROGRAM ENDED AT'`: the run was killed (OOM/timeout) or aborted
  mid-way after a nonzero soft exit.
- `energy value is not numeric`: the ENERGY line carries `NaN`/`Inf` or a
  Fortran overflow marker.
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
dual-state fixtures).
