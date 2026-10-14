package acceptance

import (
	"context"
	"fmt"
	"github.com/ujjwal6792/dots-cli/internal/distribution"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

// Upgrade requires a DIFFERENT real packaged version. It tests actual pointer
// promotion and rollback, rather than treating a no-op reinstall as an upgrade.
func Upgrade(ctx context.Context, archive, sha, updateArchive, updateSHA, dir string) error {
	var err error
	archive, err = filepath.Abs(archive)
	if err != nil {
		return err
	}
	updateArchive, err = filepath.Abs(updateArchive)
	if err != nil {
		return err
	}
	old, err := distribution.Verify(archive, sha)
	if err != nil {
		return err
	}
	next, err := distribution.Verify(updateArchive, updateSHA)
	if err != nil {
		return err
	}
	if old.Manifest.Version == next.Manifest.Version || old.Manifest.Platform != next.Manifest.Platform || next.Manifest.Platform != distribution.Platform() {
		return fmt.Errorf("upgrade acceptance needs different versions for the real host platform")
	}
	git, err := exec.LookPath("git")
	if err != nil {
		return err
	}
	h := harness{ctx, filepath.Join(dir, "extracted/dot"), git, dir, filepath.Join(dir, "path"), filepath.Join(dir, "gitconfig"), nil}
	home := filepath.Join(dir, "home-two")
	root := filepath.Join(dir, "upgrade-installation")
	if _, err := h.invoke(home, "", "install", "--root", root, "--archive", archive, "--sha256", sha, "--yes"); err != nil {
		return err
	}
	previous, err := os.Readlink(filepath.Join(root, "current"))
	if err != nil {
		return err
	}
	installed := h
	installed.dot = filepath.Join(root, "bin/dot")
	if _, err := installed.invoke(home, "", "self-update", "--root", root, "--archive", updateArchive, "--sha256", updateSHA, "--yes"); err != nil {
		return err
	}
	b, err := installed.invoke(home, "", "--version")
	if err != nil || strings.TrimSpace(string(b)) != "dot "+next.Manifest.Version {
		return fmt.Errorf("upgrade did not execute new version: %s %v", b, err)
	}
	if _, err := installed.invoke(home, "", "--health"); err != nil {
		return err
	}
	target, err := os.Readlink(filepath.Join(root, "current"))
	if err != nil {
		return err
	}
	if target == previous {
		return fmt.Errorf("upgrade did not change release pointer")
	}
	journal := map[string]any{"schema": 1, "previous": previous, "next": target}
	if err := write(filepath.Join(root, "promotion.json"), []byte(marshal(journal)), 0600); err != nil {
		return err
	}
	// A changed foreign pointer must not be overwritten during recovery.
	if err := os.Remove(filepath.Join(root, "current")); err != nil {
		return err
	}
	if err := os.Symlink("foreign-target", filepath.Join(root, "current")); err != nil {
		return err
	}
	if _, err := h.invoke(home, "", "install-recover", "--root", root, "--yes"); err == nil {
		return fmt.Errorf("recovery overwrote foreign pointer")
	}
	got, err := os.Readlink(filepath.Join(root, "current"))
	if err != nil || got != "foreign-target" {
		return fmt.Errorf("conflicting pointer changed")
	}
	if err := os.Remove(filepath.Join(root, "current")); err != nil {
		return err
	}
	if err := os.Symlink(target, filepath.Join(root, "current")); err != nil {
		return err
	}
	if _, err := installed.invoke(home, "", "install-recover", "--root", root, "--yes"); err != nil {
		return err
	}
	b, err = installed.invoke(home, "", "--version")
	if err != nil || strings.TrimSpace(string(b)) != "dot "+old.Manifest.Version {
		return fmt.Errorf("recovery did not execute previous version: %s %v", b, err)
	}
	if _, err := installed.invoke(home, "", "--health"); err != nil {
		return err
	}
	fmt.Printf("PASS: real %s upgrade %s → %s and conflict-safe rollback to %s\n", distribution.Platform(), old.Manifest.Version, next.Manifest.Version, old.Manifest.Version)
	return nil
}
