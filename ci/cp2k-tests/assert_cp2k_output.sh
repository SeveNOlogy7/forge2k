#!/usr/bin/env bash
# assert_cp2k_output.sh -- hard-gate assertions over one CP2K output file.
#
# Part of the e2e-images H2O functional suite (CPU variant
# toolchain_2026.2_generic, image cp2k/cp2k:2026.2_mpich_generic_psmp).
# Pure bash/awk/grep, no python: runs unchanged on the GitHub ubuntu runner
# and on a local Git Bash.
#
# Usage:
#   assert_cp2k_output.sh <output_file> <test_label> <exit_code>
#
# Assertions (any failure prints the concrete reason and exits 1):
#   a) exit_code == 0
#   b) <output_file> exists and is non-empty
#   c) output contains "PROGRAM ENDED AT"
#   d) at least one unified energy line is present. CP2K v2026.2 prints the
#      same line format for every force-evaluation method
#      (src/force_env_methods.F, "Print energy always in the same format for
#      all methods"):
#        ENERGY| Total FORCE_EVAL ( QS ) energy [a.u.]:   <value>
#      QS/DFT, SE (AM1) and DFTB (SCC) all run inside the QS framework
#      (use_prog_name(use_qs_force) == "QS"), so one pattern covers all four
#      suite inputs; the LAST matching line is the one checked.
#   e) that value is numeric (rejects NaN/Inf/text) and inside the label's
#      sanity interval (table below). A label may instead be PINNED to an
#      exact value +- tolerance once a CI baseline run has produced the
#      canonical number; unpinned labels pass the sanity interval alone and
#      print "UNPINNED <label> (sanity mode)".
#
# Exit codes: 0 = all assertions passed, 2 = usage error, 1 = assertion failed.

set -u

if [ "$#" -ne 3 ]; then
   echo "usage: $0 <output_file> <test_label> <exit_code>" >&2
   exit 2
fi

OUT_FILE="$1"
LABEL="$2"
EXIT_CODE="$3"

# ---------------------------------------------------------------------------
# Sanity intervals (a.u.) for the last ENERGY| value.
#   default [-100, -5]: wide bracket for the three single-molecule /
#   single-cell inputs of the suite (QS docs cell, QS OT cell, SE AM1 cell).
#   h2o-dftb-scc runs a 64-water periodic box: its SCC total energy is about
#   -130.6 a.u. (upstream reference magnitude, tests/DFTB/regtest-scc/
#   TEST_FILES.toml entry for h2o-1.inp: -130.55701170331295 -- recorded
#   here as scale evidence for the interval only, NOT as a pinned value), so
#   it gets its own wider interval.
# ---------------------------------------------------------------------------
DEFAULT_LO=-100
DEFAULT_HI=-5
declare -A SANITY_LO=( [h2o-dftb-scc]=-1000 )
declare -A SANITY_HI=( [h2o-dftb-scc]=-5 )

# ---------------------------------------------------------------------------
# Exact-value pins: LABEL -> "expected:tolerance".
# Intentionally EMPTY in v1: the suite runs in sanity mode until a CI
# baseline run fixes the canonical numbers (see README.md, "Expected
# values"). Example once pinned:
#   PINNED[h2o-quickstep]="-17.1615063648948:1e-4"
# ---------------------------------------------------------------------------
declare -A PINNED=()

fails=0
pass() { echo "ASSERT PASS [${LABEL}] $*"; }
fail() { echo "ASSERT FAIL [${LABEL}] $*" >&2; fails=$((fails + 1)); }

echo "===== assert begin: label=${LABEL} file=${OUT_FILE} exit_code=${EXIT_CODE} ====="

# (a) exit code -------------------------------------------------------------
if [ "${EXIT_CODE}" -eq 0 ] 2>/dev/null; then
   pass "exit code is 0"
else
   fail "exit code is not 0 (got: ${EXIT_CODE}) -- CP2K run itself failed; see the docker run output above"
fi

# (b) output file exists and is non-empty ----------------------------------
if [ -s "${OUT_FILE}" ]; then
   pass "output file exists and is non-empty ($(wc -c < "${OUT_FILE}" | tr -d ' ') bytes)"
else
   if [ -f "${OUT_FILE}" ]; then
      fail "output file is empty: ${OUT_FILE}"
   else
      fail "output file missing: ${OUT_FILE}"
      echo "===== assert verdict: FAIL (aborting further checks) =====" >&2
      exit 1
   fi
fi

# (c) normal termination marker --------------------------------------------
if grep -q "PROGRAM ENDED AT" "${OUT_FILE}"; then
   pass "contains 'PROGRAM ENDED AT'"
else
   fail "missing 'PROGRAM ENDED AT' (run did not terminate normally)"
fi

# (d) unified energy line + (e) value sanity / pin -------------------------
ENERGY_LINE="$(grep -E '^[[:space:]]*ENERGY\| Total FORCE_EVAL \( [A-Za-z]+ \) energy' "${OUT_FILE}" | tail -n 1)"
if [ -z "${ENERGY_LINE}" ]; then
   fail "no 'ENERGY| Total FORCE_EVAL ( ... ) energy' line found"
   VALUE=""
else
   pass "energy line found: ${ENERGY_LINE}"
   VALUE="$(printf '%s\n' "${ENERGY_LINE}" | awk '{print $NF}')"
   if printf '%s\n' "${VALUE}" | grep -Eq '^[+-]?([0-9]+(\.[0-9]*)?|\.[0-9]+)([eEdD][+-]?[0-9]+)?$'; then
      pass "energy value is numeric: ${VALUE}"
   else
      fail "energy value is not numeric (NaN/Inf/text): '${VALUE}'"
      VALUE=""
   fi
fi

if [ -n "${VALUE}" ]; then
   if [ -n "${PINNED[$LABEL]:-}" ]; then
      PIN_EXPECTED="${PINNED[$LABEL]%%:*}"
      PIN_TOL="${PINNED[$LABEL]#*:}"
      if awk -v v="${VALUE}" -v e="${PIN_EXPECTED}" -v t="${PIN_TOL}" \
            'BEGIN { d = v - e; if (d < 0) d = -d; exit !(d <= t) }'; then
         pass "pinned value within tolerance (got ${VALUE}, expected ${PIN_EXPECTED} +- ${PIN_TOL})"
      else
         fail "pinned value out of tolerance (got ${VALUE}, expected ${PIN_EXPECTED} +- ${PIN_TOL})"
      fi
   else
      echo "UNPINNED ${LABEL} (sanity mode)"
      LO="${SANITY_LO[$LABEL]:-$DEFAULT_LO}"
      HI="${SANITY_HI[$LABEL]:-$DEFAULT_HI}"
      if awk -v v="${VALUE}" -v lo="${LO}" -v hi="${HI}" \
            'BEGIN { exit !(v >= lo && v <= hi) }'; then
         pass "sanity interval ok (${VALUE} in [${LO}, ${HI}])"
      else
         fail "energy value out of sanity interval (got ${VALUE}, expected [${LO}, ${HI}])"
      fi
   fi
fi

echo "===== assert end: ${fails} failed assertion(s) ====="
if [ "${fails}" -ne 0 ]; then
   echo "ASSERT VERDICT: FAIL [${LABEL}]" >&2
   exit 1
fi
echo "ASSERT VERDICT: PASS [${LABEL}]"
exit 0
