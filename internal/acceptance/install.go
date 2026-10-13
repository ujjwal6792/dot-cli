package acceptance

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
)

func (h harness) initialInstallRecovery(home, archive, checksum string) error {
	root := filepath.Join(h.dir, "initial-install-recovery")
	if _, err := h.invoke(home, "", "install", "--root", root, "--archive", archive, "--sha256", checksum, "--yes"); err != nil {
		return err
	}
	target, err := os.Readlink(filepath.Join(root, "current"))
	if err != nil {
		return err
	}
	owner, err := os.ReadFile(filepath.Join(root, "owner.json"))
	if err != nil {
		return err
	}
	// External durable initial-promotion interruption fixture. Recovery is the real
	// packaged command, not an injected engine success or test-only install switch.
	if err = write(filepath.Join(root, "promotion.json"), []byte(marshal(map[string]any{"schema": 1, "previous": "", "next": target})), 0600); err != nil {
		return err
	}
	if _, err = h.invoke(home, "", "self-update", "--root", root, "--archive", archive, "--sha256", checksum, "--yes"); err == nil {
		return fmt.Errorf("pending initial promotion allowed update")
	}
	if _, err = h.invoke(home, "", "install-recover", "--root", root, "--yes"); err != nil {
		return err
	}
	for _, path := range []string{"current", "bin/dot", "promotion.json"} {
		if _, err = os.Lstat(filepath.Join(root, path)); !os.IsNotExist(err) {
			return fmt.Errorf("initial rollback retained %s", path)
		}
	}
	after, err := os.ReadFile(filepath.Join(root, "owner.json"))
	if err != nil || !bytes.Equal(owner, after) {
		return fmt.Errorf("initial rollback changed owner")
	}
	if _, err = os.Stat(filepath.Join(root, target, "dot")); err != nil {
		return fmt.Errorf("initial rollback deleted retained release")
	}
	foreign := filepath.Join(h.dir, "foreign-manager-owned")
	if err = write(filepath.Join(foreign, "bin/dot"), []byte("manager-owned fixture"), 0755); err != nil {
		return err
	}
	for _, args := range [][]string{{"uninstall", "--root", foreign, "--yes"}, {"install", "--root", foreign, "--archive", archive, "--sha256", checksum, "--yes"}} {
		if _, err = h.invoke(home, "", args...); err == nil {
			return fmt.Errorf("foreign manager-owned root accepted")
		}
	}
	b, err := os.ReadFile(filepath.Join(foreign, "bin/dot"))
	if err != nil || string(b) != "manager-owned fixture" {
		return fmt.Errorf("foreign manager-owned installation modified")
	}
	return nil
}
