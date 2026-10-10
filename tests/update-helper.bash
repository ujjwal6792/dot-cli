# Upgrade only the update fixtures to the independent installed contract.
independent_update_engine() {
  export DOT_ENGINE="$BATS_TEST_TMPDIR/public/tools/dot"
  mkdir -p "$DOT_ENGINE"
  cp "$REPO/tools/dot/"*.py "$REPO/tools/dot/engine.json" "$REPO/tools/dot/dot" "$DOT_ENGINE/"
  python3 - "$DOT_CONFIG" "$DOT_ENGINE" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
lines = [line for line in path.read_text().splitlines() if not line.startswith('dotEngine=')]
lines.append('dotEngine=' + __import__('json').dumps(sys.argv[2]))
path.write_text('\n'.join(lines) + '\n')
PY
  # DOT_SOURCE already carries a public synthetic contract from setup. Do not
  # import the private repository's descriptor as a fixture dependency.
  DOT="$DOT_ENGINE/dot"
}
