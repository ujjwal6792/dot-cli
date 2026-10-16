package acceptance

import (
	"bytes"
	"context"
	"fmt"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"time"
)

// openApp talks directly to a native pseudoterminal. No script(1), shell or
// model mock launches the app; a rendered page and normal quit are required.
func (h harness) openApp(home, want string, args ...string) error {
	label := "open-" + sanitizeOpenLabel(want+"-"+strings.Join(args, "-"))
	if label == "open--" {
		label = "open-app"
	}
	ctx, cancel := context.WithTimeout(h.ctx, 15*time.Second)
	defer cancel()
	master, slave, err := openPTY()
	if err != nil {
		return err
	}
	defer master.Close()
	defer slave.Close()
	c := exec.CommandContext(ctx, h.dot, args...)
	c.Dir = h.dir
	c.Env = h.env(home)
	c.Stdin = slave
	c.Stdout = slave
	c.Stderr = slave
	c.SysProcAttr = &syscall.SysProcAttr{Setsid: true, Setctty: true, Ctty: 0}
	if err := c.Start(); err != nil {
		return err
	}
	slave.Close()
	ready := make(chan struct{}, 1)
	transcript := make(chan []byte, 1)
	go func() {
		var out bytes.Buffer
		found := false
		buffer := make([]byte, 4096)
		for {
			n, err := master.Read(buffer)
			if n > 0 {
				if out.Len() < 1<<20 {
					out.Write(buffer[:n])
				}
				if !found && bytes.Contains(out.Bytes(), []byte(want)) {
					ready <- struct{}{}
					found = true
				}
			}
			if err != nil {
				transcript <- out.Bytes()
				return
			}
		}
	}()
	select {
	case <-ready:
	case <-ctx.Done():
		_ = c.Wait()
		master.Close()
		b := <-transcript
		_ = write(filepath.Join(h.dir, label+".terminal"), b, 0600)
		return fmt.Errorf("packaged app %v never rendered %q: %s", args, want, b)
	}
	if _, err := master.Write([]byte("q")); err != nil {
		cancel()
		_ = c.Wait()
		return err
	}
	err = c.Wait()
	master.Close()
	b := <-transcript
	_ = write(filepath.Join(h.dir, label+".terminal"), b, 0600)
	if err != nil {
		return fmt.Errorf("packaged app %v quit failed: %w: %s", args, err, b)
	}
	return nil
}

func sanitizeOpenLabel(s string) string {
	var b strings.Builder
	for _, r := range strings.ToLower(s) {
		if (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9') {
			b.WriteRune(r)
		} else {
			b.WriteRune('-')
		}
	}
	out := strings.Trim(b.String(), "-")
	for strings.Contains(out, "--") {
		out = strings.ReplaceAll(out, "--", "-")
	}
	if out == "" {
		return "app"
	}
	return out
}
