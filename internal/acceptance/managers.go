package acceptance

import (
	"bytes"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
)

// A deliberately non-installing native child fixture. It exercises the real
// packaged terminal/manager boundary, never impersonates successful dependency
// provisioning and cannot invoke a host manager or sudo.
const managerFixture = `package main
import("bufio";"encoding/json";"fmt";"os";"path/filepath";"strings";"time")
func main(){
 b,_:=json.Marshal(map[string]any{"args":os.Args[1:],"no_auto_update":os.Getenv("HOMEBREW_NO_AUTO_UPDATE"),"no_upgrade":os.Getenv("HOMEBREW_NO_INSTALL_UPGRADE")})
 os.WriteFile(filepath.Join(os.Getenv("HOME"),"manager-call.json"),b,0600)
 if strings.Contains(os.Getenv("HOME"),"home-source-interrupt") { fmt.Fprintln(os.Stderr,"ISOLATED MANAGER: enter fixture response"); time.Sleep(time.Hour); return }
 fmt.Fprintln(os.Stdout,"ISOLATED MANAGER: enter fixture response")
 line,_:=bufio.NewReader(os.Stdin).ReadString('\n')
 fmt.Fprintln(os.Stderr,"ISOLATED MANAGER stderr received",line)
}
`

func (h harness) managerCases() error {
	if runtime.GOOS != "darwin" {
		return fmt.Errorf("Darwin manager fixtures require Darwin runtime")
	}
	dir := filepath.Join(h.dir, "manager-fixture")
	if err := os.Mkdir(dir, 0700); err != nil {
		return err
	}
	src := filepath.Join(dir, "main.go")
	if err := write(src, []byte(managerFixture), 0600); err != nil {
		return err
	}
	// Maintainer-only fixture compilation occurs before the child app runs.
	// Neither Go nor a host manager enters the app's isolated PATH.
	build := exec.CommandContext(h.ctx, "go", "build", "-o", filepath.Join(dir, "brew"), src)
	if b, err := build.CombinedOutput(); err != nil {
		return fmt.Errorf("build isolated manager: %w %s", err, b)
	}
	fixture := h
	fixture.path = dir
	for _, kind := range []string{"skip", "zero", "interrupt"} {
		home := filepath.Join(h.dir, "home-manager-"+kind)
		if err := os.Mkdir(home, 0700); err != nil {
			return err
		}
		steps := []keyboardStep{{"", "Welcome"}, {"\r", "https://git-scm.com/"}}
		exit := 0
		switch kind {
		case "skip":
			steps = append(steps, keyboardStep{" ", "Setup incomplete"}, keyboardStep{"\r", ""})
		case "zero":
			steps = append(steps, keyboardStep{"\r", "ISOLATED MANAGER: enter fixture response"}, keyboardStep{"fixture-answer\r", "required dependencies still missing"}, keyboardStep{"q", ""})
		case "interrupt":
			exit = 130
			steps = append(steps, keyboardStep{"\r", "ISOLATED MANAGER: enter fixture response"}, keyboardStep{"\x03", ""})
		}
		if err := fixture.keyboardExit(home, "keyboard-manager-"+kind, exit, steps); err != nil {
			return err
		}
		b, err := os.ReadFile(filepath.Join(home, "manager-call.json"))
		if kind == "skip" {
			if !os.IsNotExist(err) {
				return fmt.Errorf("Space skip invoked manager")
			}
		} else {
			if err != nil || !bytes.Contains(b, []byte(`"args":["install","--formula","--","git"]`)) || !bytes.Contains(b, []byte(`"no_upgrade":"1"`)) {
				return fmt.Errorf("native manager arguments/ownership environment: %s %v", b, err)
			}
		}
		if _, err := os.Stat(filepath.Join(home, ".local")); !os.IsNotExist(err) {
			return fmt.Errorf("missing/skipped/interrupted dependencies wrote source/state")
		}
	}
	// Real system Git delegates SSH authentication to an isolated blocked native
	// child. Interrupt the real clone while its terminal is released; this is not
	// a mocked engine/source success and no remote is contacted.
	home := filepath.Join(h.dir, "home-source-interrupt")
	if err := os.Mkdir(home, 0700); err != nil {
		return err
	}
	sourceFixture := h
	sourceFixture.extra = []string{"GIT_SSH=" + filepath.Join(dir, "brew"), "GIT_SSH_VARIANT=ssh"}
	steps := []keyboardStep{{"", "Welcome"}, {"\r", "Existing user"}, {"\r", "Input hidden"}, {"git@isolated.invalid:config.git\r", "ISOLATED MANAGER: enter fixture response"}, {"\x03", ""}}
	if err := sourceFixture.keyboardExit(home, "keyboard-source-interrupt", 130, steps); err != nil {
		return err
	}
	if _, err := os.Stat(filepath.Join(home, ".local/state/dot/connection.json")); !os.IsNotExist(err) {
		return fmt.Errorf("interrupted clone published connection")
	}
	if _, err := os.Stat(filepath.Join(home, ".local/state/dot/adoption.json")); !os.IsNotExist(err) {
		return fmt.Errorf("interrupted clone published identity")
	}
	return nil
}
