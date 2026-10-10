#!/usr/bin/env bats

@test "private package planner supports all OS fixtures with no host manager queries or real installs" {
  run python3 -B "$BATS_TEST_DIRNAME/packages-test.py"
  [ "$status" -eq 0 ]
  [[ "$output" == *'OK'* ]]
}
