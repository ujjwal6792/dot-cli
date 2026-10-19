package acceptance

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	"github.com/ujjwal6792/dots-cli/internal/native"
)

type keyboardStep struct{ keys, want string }

// keyboard executes the extracted application, not a model or engine double.
// Each input waits for a newly rendered result; transcript evidence survives errors.
func (h harness) keyboard(home, label string, steps []keyboardStep, args ...string) error {
	return h.keyboardExit(home, label, 0, steps, args...)
}

func (h harness) keyboardExit(home, label string, expectedExit int, steps []keyboardStep, args ...string) error {
	ctx, cancel := context.WithTimeout(h.ctx, 45*time.Second)
	defer cancel()
	master, slave, err := openPTY()
	if err != nil {
		return err
	}
	defer master.Close()
	defer slave.Close()
	cmd := exec.CommandContext(ctx, h.dot, args...)
	cmd.Dir = h.dir
	cmd.Env = h.env(home)
	cmd.Stdin = slave
	cmd.Stdout = slave
	cmd.Stderr = slave
	cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true, Setctty: true, Ctty: 0}
	if err = cmd.Start(); err != nil {
		return err
	}
	slave.Close()
	chunks := make(chan []byte, 256)
	go func() {
		defer close(chunks)
		buf := make([]byte, 4096)
		for {
			n, e := master.Read(buf)
			if n > 0 {
				chunks <- append([]byte(nil), buf[:n]...)
			}
			if e != nil {
				return
			}
		}
	}()
	var transcript bytes.Buffer
	screen := newTerminalScreen(100, 32)
	defer func() { _ = write(filepath.Join(h.dir, label+".terminal"), transcript.Bytes(), 0600) }()
	for i, s := range steps {
		if s.keys != "" {
			if _, err = master.Write([]byte(s.keys)); err != nil {
				cancel()
				_ = cmd.Wait()
				return err
			}
		}
		if s.want == "" {
			continue
		}
		found := false
		for !found {
			select {
			case b, ok := <-chunks:
				if !ok {
					_ = cmd.Wait()
					return fmt.Errorf("%s step %d exited before %q: %s", label, i, s.want, transcript.String())
				}
				transcript.Write(b)
				screen.write(b)
				found = strings.Contains(screen.text(), s.want)
			case <-ctx.Done():
				_ = cmd.Wait()
				master.Close()
				return fmt.Errorf("%s step %d never rendered %q: %s", label, i, s.want, transcript.String())
			}
		}
	}
	err = cmd.Wait()
	master.Close()
	for b := range chunks {
		transcript.Write(b)
	}
	exit := 0
	if err != nil {
		if e, ok := err.(*exec.ExitError); ok {
			exit = e.ExitCode()
		} else {
			return err
		}
	}
	if exit != expectedExit {
		return fmt.Errorf("%s keyboard exit %d, want %d: %s", label, exit, expectedExit, transcript.String())
	}
	return nil
}

func (h harness) interruptKeyboard() error {
	home := filepath.Join(h.dir, "home-interrupt")
	if err := os.Mkdir(home, 0700); err != nil {
		return err
	}
	if err := h.keyboardExit(home, "keyboard-interrupt", 130, []keyboardStep{{"", "Welcome"}, {"\x03", ""}}); err != nil {
		return err
	}
	if _, err := os.Stat(filepath.Join(home, ".local")); !os.IsNotExist(err) {
		return fmt.Errorf("Ctrl+C before source wrote private state")
	}
	return nil
}

func setupTail(id string) []keyboardStep {
	return []keyboardStep{
		{"\x15" + id + "\r", "Nothing to install."},
		{"\r", "tracker needs name and email"},
		{"Keyboard User\tkeyboard@example.invalid\r", "Confirm tracker and connection"},
		{"\r", "Connection consent canceled"}, // Cancel is the default.
		{"\r", "Confirm tracker and connection"},
		{"j\r", "No files adopted"},
		{"\r", "Tracked Files"},
		{"q", ""},
	}
}

func (h harness) setupKeyboard(home, url, id, action string) error {
	steps := []keyboardStep{{"", "Welcome"}, {"\r", "Existing user"}}
	if action == "new" {
		steps = append(steps, keyboardStep{"j\r", "Create the empty repository"})
	} else {
		steps = append(steps, keyboardStep{"\r", "Input hidden"})
	}
	steps = append(steps, keyboardStep{"\x16", "terminal paste/copy shortcut"})
	steps = append(steps, keyboardStep{"\x1b[200~" + url + "\x1b[201~\r", "Source connected and contract-checked"})
	steps = append(steps, setupTail(id)...)
	if err := h.keyboard(home, "keyboard-"+id, steps); err != nil {
		return err
	}
	return h.verifySetup(home, id)
}

func (h harness) verifySetup(home, id string) error {
	b, err := os.ReadFile(filepath.Join(home, ".local/state/dot/adoption.json"))
	if err != nil {
		return err
	}
	var a native.Adoption
	if err = native.Decode(b, &a); err != nil {
		return err
	}
	if a.Machine.ID != id || len(a.Entries) != 0 {
		return fmt.Errorf("keyboard setup identity/empty scope: %+v", a)
	}
	_, err = h.invoke(home, "", "init", "--complete")
	return err
}

func (h harness) continueReplace(url string) error {
	home := filepath.Join(h.dir, "home-continue-replace")
	if err := os.Mkdir(home, 0700); err != nil {
		return err
	}
	if _, err := h.invoke(home, "", "init", "--action", "new", "--url", url); err != nil {
		return err
	}
	source := filepath.Join(home, ".local/share/dot-config")
	if err := h.gitRun(home, "-C", source, "add", "."); err != nil {
		return err
	}
	if err := h.gitRun(home, "-C", source, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-m", "before reviewed replacement"); err != nil {
		return err
	}
	manifestPath := filepath.Join(source, "manifest.json")
	dirtyManifest, err := os.ReadFile(manifestPath)
	if err != nil {
		return err
	}
	dirtyManifest = append(dirtyManifest, []byte("\n \n")...)
	if err = write(manifestPath, dirtyManifest, 0600); err != nil {
		return err
	}
	dirty := []byte("dirty and untracked\x00\xff\n")
	if err := write(filepath.Join(source, "untracked"), dirty, 0751); err != nil {
		return err
	}
	// Continue really validates the existing source and allows edited identity.
	steps := []keyboardStep{{"", "Welcome"}, {"\r", "Continue"}, {"\r", "Source connected and contract-checked"}, {"\x15macos_continue\r", "Nothing to install."}, {"\r", "tracker needs name and email"}, {"\x1b", "Dependencies"}, {"q", ""}}
	if err := h.keyboard(home, "keyboard-continue", steps, "init"); err != nil {
		return err
	}
	// No tracker/connection: reopening still gives the setup/Replace path.
	steps = []keyboardStep{{"", "Welcome"}, {"\r", "Continue"}, {"j\r", "Confirm backup and replace"}, {"\r", "Continue"}, {"j\r", "Confirm backup and replace"}, {"j\r", "Original checkout preserved"}, {"j\r", "Create the empty repository"}, {url + "\r", "Source connected and contract-checked"}}
	// Saved identity is immutable, so retain it explicitly after replacement.
	steps = append(steps, setupTail("macos_continue")...)
	if err := h.keyboard(home, "keyboard-replace", steps, "init"); err != nil {
		return err
	}
	backups, err := filepath.Glob(source + ".backup-*/checkout")
	if err != nil || len(backups) != 1 {
		return fmt.Errorf("Replace backup count: %v %v", backups, err)
	}
	b, err := os.ReadFile(filepath.Join(backups[0], "untracked"))
	if err != nil || !bytes.Equal(b, dirty) {
		return fmt.Errorf("Replace lost untracked bytes")
	}
	info, err := os.Stat(filepath.Join(backups[0], "untracked"))
	if err != nil || info.Mode().Perm() != 0751 {
		return fmt.Errorf("Replace lost untracked mode")
	}
	if _, err = os.Stat(filepath.Join(backups[0], ".git")); err != nil {
		return fmt.Errorf("Replace lost Git: %w", err)
	}
	b, err = os.ReadFile(filepath.Join(backups[0], "manifest.json"))
	if err != nil || !bytes.Equal(b, dirtyManifest) {
		return fmt.Errorf("Replace lost dirty tracked bytes")
	}
	return h.verifySetup(home, "macos_continue")
}

func (h harness) trackKeyboard(home string) error {
	if err := write(filepath.Join(home, "keyboard-prefs"), []byte("keyboard exact\x00\xff\n"), 0755); err != nil {
		return err
	}
	steps := []keyboardStep{
		{"", "Tracked Files"}, {"2", "0 selected"}, {"/keyboard-prefs\r", "keyboard-prefs"},
		{" ", "EDIT TRACK METADATA"}, {"\x1b", "Search results"},
		{" ", "EDIT TRACK METADATA"}, {"\t   \t \t \r", "Exact selection metadata saved"},
		{"M", "LOGICAL FILE METADATA"}, {"\x13", "Logical metadata request prepared"},
		{"p", "a continue"}, {"a", "FINAL CONFIRMATION"}, {"\r", "Final confirmation canceled"},
		{"a", "FINAL CONFIRMATION"}, {"j\r", "Applied reviewed plan."}, {"q", ""},
	}
	if err := h.keyboard(home, "keyboard-track", steps); err != nil {
		return err
	}
	a, err := h.scope(home)
	if err != nil || len(a.Entries) != 0 {
		return fmt.Errorf("keyboard Track adopted: %v", err)
	}
	return nil
}

func (h harness) adoptKeyboard(home string) error {
	steps := []keyboardStep{{"", "Tracked Files"}, {"3", "0 selected"}, {" ", "x] keyboard-prefs"}, {"p", "a continue"}, {"a", "FINAL CONFIRMATION"}, {"\r", "Final confirmation canceled"}, {"a", "FINAL CONFIRMATION"}, {"j\r", "Applied reviewed plan."}, {"q", ""}}
	if err := h.keyboard(home, "keyboard-adopt", steps); err != nil {
		return err
	}
	b, err := os.ReadFile(filepath.Join(home, "keyboard-prefs"))
	if err != nil || string(b) != "keyboard exact\x00\xff\n" {
		return fmt.Errorf("keyboard Adopt bytes mismatch: %v", err)
	}
	st, err := os.Stat(filepath.Join(home, "keyboard-prefs"))
	if err != nil || st.Mode().Perm() != 0755 {
		return fmt.Errorf("keyboard Adopt executable mode mismatch")
	}
	return nil
}

// navigationKeyboard sends the bytes emitted by a terminal for Tab/Backtab.
// Pane-specific text proves focus moved; every tab name is always in the header.
func (h harness) navigationKeyboard(home string) error {
	steps := []keyboardStep{{"", "Tracked Files"}, {" ", "Status & diff · expanded"}, {" ", "Tracked Files"}}
	for _, want := range []string{"TRACK", "ADOPT", "UNTRACK", "GIT SYNC · Explicit Gates", "Find a setting", "Tracked Files"} {
		steps = append(steps, keyboardStep{"\t", want})
	}
	for _, want := range []string{"Find a setting", "GIT SYNC · Explicit Gates", "UNTRACK", "ADOPT", "TRACK", "Tracked Files"} {
		steps = append(steps, keyboardStep{"\x1b[Z", want})
	}
	steps = append(steps,
		keyboardStep{"6", "Find a setting"},
		keyboardStep{"/q123456789", "No matching settings"}, // q is text, not quit, in the input.
		keyboardStep{"\x1b", "Search sections"},
		keyboardStep{"\x0e\r", "Dependencies re-verified"}, // Ctrl+N focuses Verify.
		keyboardStep{"T", "Contract valid"},
		keyboardStep{"q", ""},
	)
	return h.keyboard(home, "keyboard-navigation", steps)
}

func pointerPacket(x, y int) string {
	return fmt.Sprintf("\x1b[<0;%d;%dM\x1b[<0;%d;%dm", x+1, y+1, x+1, y+1)
}

// Real SGR pointer packets exercise terminal decoding, not only model messages.
// This journey is read-only: selecting an option is canceled before review.
func (h harness) mouseJourney(home string) error {
	path := filepath.Join(home, "pointer-prefs")
	if err := write(path, []byte("pointer fixture\n"), 0600); err != nil {
		return err
	}
	defer os.Remove(path)
	steps := []keyboardStep{
		{"", "Tracked Files"},
		{pointerPacket(23, 1), "TRACK"},
		{pointerPacket(30, 3), "/ filter"},
		{"pointer-prefs", "Search results · pointer-prefs"},
		{pointerPacket(29, 4), "EDIT TRACK METADATA"},
		{pointerPacket(25, 6), "[darwin]"},
		{pointerPacket(37, 30), "Search results"},
		{pointerPacket(6, 1), "Read-only inspection"},
		{pointerPacket(4, 2), "Status & diff · expanded"},
		{pointerPacket(4, 2), "Search results"},
		{"q", ""},
	}
	if err := h.keyboard(home, "mouse-workspace", steps); err != nil {
		return err
	}
	transcript, err := os.ReadFile(filepath.Join(h.dir, "mouse-workspace.terminal"))
	if err != nil {
		return err
	}
	if !bytes.Contains(transcript, []byte("\x1b[?1002h")) || !bytes.Contains(transcript, []byte("\x1b[?1006h")) {
		return fmt.Errorf("app did not request cell-motion and SGR mouse reporting")
	}
	a, err := h.scope(home)
	if err != nil || len(a.Entries) != 0 {
		return fmt.Errorf("pointer browsing changed adoption scope: %v", err)
	}
	if _, err := os.Stat(filepath.Join(home, ".local/share/dot-config/home/pointer-prefs")); !os.IsNotExist(err) {
		return fmt.Errorf("pointer browsing wrote a source file")
	}
	return nil
}

func (h harness) largeBrowserKeyboard(home string) error {
	root := filepath.Join(home, "zz-large-list")
	if err := os.Mkdir(root, 0700); err != nil {
		return err
	}
	defer os.RemoveAll(root)
	nested := filepath.Join(root, "000-collapsed")
	if err := os.Mkdir(nested, 0700); err != nil {
		return err
	}
	for i := range 1000 {
		if err := write(filepath.Join(root, fmt.Sprintf("entry-%04d", i)), []byte("fixture\n"), 0600); err != nil {
			return err
		}
	}
	for i := range 9000 {
		if err := write(filepath.Join(nested, fmt.Sprintf("nested-%04d", i)), []byte("fixture\n"), 0600); err != nil {
			return err
		}
	}
	started := time.Now()
	err := h.keyboard(home, "keyboard-large-list", []keyboardStep{
		{"", "Tracked Files"}, {"2", "0 selected"},
		{"/zz-large-list\r", "zz-large-list"}, {"\r", "entry-0000"},
		{"\x1b[F", "entry-0999"}, {"\x1b[H", "000-collapsed"}, {"h", ".config"}, {"q", ""},
	})
	if err != nil {
		return err
	}
	if err := write(filepath.Join(h.dir, "large-browser-duration.txt"), []byte(time.Since(started).String()+"\n"), 0600); err != nil {
		return err
	}
	a, err := h.scope(home)
	if err != nil || len(a.Entries) != 0 {
		return fmt.Errorf("large-list browsing changed approval: %v", err)
	}
	if _, err := os.Stat(filepath.Join(home, ".local/share/dot-config/home/zz-large-list")); !os.IsNotExist(err) {
		return fmt.Errorf("large-list browsing wrote source")
	}
	return nil
}
