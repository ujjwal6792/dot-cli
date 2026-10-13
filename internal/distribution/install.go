package distribution

import (
	"encoding/json"
	"fmt"
	"github.com/ujjwal6792/dots-cli/internal/native"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"syscall"
)

type owner struct {
	Schema int    `json:"schema"`
	Owner  string `json:"owner"`
	Root   string `json:"root"`
	UID    int    `json:"uid"`
}
type promotion struct {
	Schema   int    `json:"schema"`
	Previous string `json:"previous"`
	Next     string `json:"next"`
}

func safeRoot(root string) error {
	if !filepath.IsAbs(root) || filepath.Clean(root) != root || root == "/" {
		return fmt.Errorf("private clean absolute installation root required")
	}
	for _, p := range []string{"/usr", "/bin", "/sbin", "/opt/homebrew", "/opt/local", "/Library", "/System", "/etc"} {
		if root == p || strings.HasPrefix(root, p+"/") {
			return fmt.Errorf("system/package-manager-owned paths cannot be installed, updated or uninstalled by dot")
		}
	}
	_, err := native.BoundedPath(filepath.Dir(root), filepath.Base(root))
	return err
}
func readOwner(root string) error {
	if err := safeRoot(root); err != nil {
		return err
	}
	p, err := native.BoundedPath(root, "owner.json")
	if err != nil {
		return err
	}
	ownerStat, err := os.Lstat(p)
	if err != nil {
		return fmt.Errorf("not a dot-owned installation: %w", err)
	}
	if !ownerStat.Mode().IsRegular() || ownerStat.Mode().Perm() != 0600 || ownerStat.Size() > 4096 {
		return fmt.Errorf("invalid installation owner file mode/type/size")
	}
	if s, ok := ownerStat.Sys().(*syscall.Stat_t); !ok || int(s.Uid) != os.Geteuid() {
		return fmt.Errorf("installation owner record belongs to another user")
	}
	b, err := os.ReadFile(p)
	if err != nil {
		return fmt.Errorf("not a dot-owned installation: %w", err)
	}
	var o owner
	if err := native.Decode(b, &o); err != nil {
		return fmt.Errorf("invalid installation owner")
	}
	if o != (owner{1, "dot-native", root, os.Geteuid()}) {
		return fmt.Errorf("installation ownership conflict; use its original package manager")
	}
	st, err := os.Lstat(root)
	if err != nil {
		return err
	}
	if !st.IsDir() {
		return fmt.Errorf("installation root is not a directory")
	}
	if st.Mode().Perm()&0022 != 0 {
		return fmt.Errorf("installation root must not be writable by group/other")
	}
	if s, ok := st.Sys().(*syscall.Stat_t); !ok || int(s.Uid) != os.Geteuid() {
		return fmt.Errorf("installation filesystem owner conflict")
	}
	return nil
}
func initialize(root string) error {
	if err := safeRoot(root); err != nil {
		return err
	}
	entries, err := os.ReadDir(root)
	if err == nil && len(entries) > 0 {
		return readOwner(root)
	}
	if err != nil && !os.IsNotExist(err) {
		return err
	}
	if err := os.MkdirAll(root, 0700); err != nil {
		return err
	}
	if err := safeRoot(root); err != nil {
		return err
	}
	b, _ := json.Marshal(owner{1, "dot-native", root, os.Geteuid()})
	return writeExclusive(filepath.Join(root, "owner.json"), b, 0600)
}
func lock(root string) (func(), error) {
	if err := readOwner(root); err != nil {
		return nil, err
	}
	p, err := native.BoundedPath(root, ".lock")
	if err != nil {
		return nil, err
	}
	f, err := os.OpenFile(p, os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return nil, err
	}
	if err := syscall.Flock(int(f.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		f.Close()
		return nil, fmt.Errorf("another installation operation is active")
	}
	return func() { _ = syscall.Flock(int(f.Fd()), syscall.LOCK_UN); _ = f.Close() }, nil
}
func writeExclusive(path string, b []byte, mode os.FileMode) error {
	f, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, mode)
	if err != nil {
		return err
	}
	if err = f.Chmod(mode); err == nil {
		_, err = f.Write(b)
	}
	if err == nil {
		err = f.Sync()
	}
	ce := f.Close()
	if err != nil {
		return err
	}
	if ce != nil {
		return ce
	}
	return syncDir(filepath.Dir(path))
}
func syncDir(path string) error {
	f, err := os.Open(path)
	if err != nil {
		return err
	}
	defer f.Close()
	return f.Sync()
}
func current(root string) (string, error) {
	p := filepath.Join(root, "current")
	v, err := os.Readlink(p)
	if os.IsNotExist(err) {
		return "", nil
	}
	if err != nil {
		return "", fmt.Errorf("current is not an owned symlink")
	}
	if !validTarget(v) {
		return "", fmt.Errorf("invalid current release target")
	}
	if _, err := ReadBundle(filepath.Join(root, v)); err != nil {
		return "", fmt.Errorf("current release invalid: %w", err)
	}
	return v, nil
}
func validTarget(v string) bool { return regexp.MustCompile(`^releases/[a-f0-9]{64}$`).MatchString(v) }
func link(root, target string) error {
	// A reserved temporary directory prevents symlink races on a predictable name.
	d, err := os.MkdirTemp(root, ".pointer-")
	if err != nil {
		return err
	}
	defer os.Remove(d)
	tmp := filepath.Join(d, "current")
	if err := os.Symlink(target, tmp); err != nil {
		return err
	}
	if err := syncDir(d); err != nil {
		return err
	}
	if err := os.Rename(tmp, filepath.Join(root, "current")); err != nil {
		return err
	}
	return syncDir(root)
}
func launch(root string) error {
	p, err := native.BoundedPath(root, "bin")
	if err != nil {
		return err
	}
	if err := os.MkdirAll(p, 0755); err != nil {
		return err
	}
	file := filepath.Join(p, "dot")
	v, err := os.Readlink(file)
	if err == nil {
		if v != "../current/dot" {
			return fmt.Errorf("foreign launch link; nothing overwritten")
		}
		return nil
	}
	if !os.IsNotExist(err) {
		return fmt.Errorf("foreign launch file; nothing overwritten")
	}
	if err := os.Symlink("../current/dot", file); err != nil {
		return err
	}
	return syncDir(p)
}
func checkLaunch(root string) error {
	bin, err := native.BoundedPath(root, "bin")
	if err != nil {
		return err
	}
	v, err := os.Readlink(filepath.Join(bin, "dot"))
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil || v != "../current/dot" {
		return fmt.Errorf("foreign launch file/link; nothing overwritten")
	}
	return nil
}
func pending(root string) error {
	p, err := native.BoundedPath(root, "promotion.json")
	if err != nil {
		return err
	}
	if _, err := os.Lstat(p); err == nil {
		return fmt.Errorf("interrupted promotion; run install-recover before another operation")
	} else if !os.IsNotExist(err) {
		return err
	}
	return nil
}

// Install verifies everything before reserving/publishing immutable bytes. It
// never replaces a system executable and never reads private connection state.
func Install(root, archive, checksum string, update, consent bool) error {
	if !consent {
		return fmt.Errorf("explicit --yes installation consent required")
	}
	a, err := Verify(archive, checksum)
	if err != nil {
		return err
	}
	if a.Manifest.Platform != Platform() {
		return fmt.Errorf("archive platform differs from host")
	}
	if update {
		if err := readOwner(root); err != nil {
			return err
		}
	} else {
		if err := initialize(root); err != nil {
			return err
		}
	}
	unlock, err := lock(root)
	if err != nil {
		return err
	}
	defer unlock()
	if err := pending(root); err != nil {
		return err
	}
	prev, err := current(root)
	if err != nil {
		return err
	}
	if update && prev == "" {
		return fmt.Errorf("no active installation to update")
	}
	if !update && prev != "" {
		return fmt.Errorf("already installed; use self-update")
	}
	// Check launch ownership before publishing a transaction.
	if err := checkLaunch(root); err != nil {
		return err
	}
	releases, err := native.BoundedPath(root, "releases")
	if err != nil {
		return err
	}
	if err := os.MkdirAll(releases, 0700); err != nil {
		return err
	}
	next := "releases/" + checksum
	dest := filepath.Join(root, next)
	if _, err := os.Lstat(dest); err == nil {
		m, err := ReadBundle(dest)
		if err != nil {
			return err
		}
		if m.Version != a.Manifest.Version {
			return fmt.Errorf("immutable release collision")
		}
		for name, want := range a.Files {
			b, err := os.ReadFile(filepath.Join(dest, name))
			if err != nil {
				return err
			}
			if Hash(b) != Hash(want) {
				return fmt.Errorf("immutable release bytes conflict: %s", name)
			}
		}
	} else if !os.IsNotExist(err) {
		return err
	} else {
		stage, err := os.MkdirTemp(releases, ".stage-")
		if err != nil {
			return err
		}
		// Failed stages are intentionally retained for diagnosis; no broad deletion.
		for name, b := range a.Files {
			p := filepath.Join(stage, name)
			if err := os.MkdirAll(filepath.Dir(p), 0755); err != nil {
				return err
			}
			if err := writeExclusive(p, b, os.FileMode(modes[name])); err != nil {
				return err
			}
		}
		if _, err := ReadBundle(stage); err != nil {
			return err
		}
		if err := syncDir(stage); err != nil {
			return err
		}
		if err := os.Rename(stage, dest); err != nil {
			return err
		}
		if err := syncDir(releases); err != nil {
			return err
		}
	}
	journal, _ := json.Marshal(promotion{1, prev, next})
	if err := writeExclusive(filepath.Join(root, "promotion.json"), journal, 0600); err != nil {
		return err
	}
	if err := launch(root); err != nil {
		return err
	}
	if err := link(root, next); err != nil {
		return err
	}
	if err := os.Remove(filepath.Join(root, "promotion.json")); err != nil {
		return err
	}
	return syncDir(root)
}

// RecoverInstall rolls back only a coherent pending pointer promotion; it will
// not replace links changed by a human or another installation.
func RecoverInstall(root string, consent bool) error {
	if !consent {
		return fmt.Errorf("explicit --yes recovery consent required")
	}
	unlock, err := lock(root)
	if err != nil {
		return err
	}
	defer unlock()
	p, err := native.BoundedPath(root, "promotion.json")
	if err != nil {
		return err
	}
	b, err := os.ReadFile(p)
	if err != nil {
		return err
	}
	var j promotion
	if err := native.Decode(b, &j); err != nil {
		return err
	}
	if j.Schema != 1 || !validTarget(j.Next) || (j.Previous != "" && !validTarget(j.Previous)) {
		return fmt.Errorf("invalid promotion journal")
	}
	if _, err := ReadBundle(filepath.Join(root, j.Next)); err != nil {
		return err
	}
	cur, err := current(root)
	if err != nil {
		return err
	}
	if cur != j.Next && cur != j.Previous {
		return fmt.Errorf("promotion recovery conflict")
	}
	if err := checkLaunch(root); err != nil {
		return err
	}
	if j.Previous != "" {
		if _, err := ReadBundle(filepath.Join(root, j.Previous)); err != nil {
			return err
		}
		if err := link(root, j.Previous); err != nil {
			return err
		}
	} else if cur != "" {
		if err := os.Remove(filepath.Join(root, "current")); err != nil {
			return err
		}
	}
	if j.Previous == "" {
		bin, err := native.BoundedPath(root, "bin")
		if err != nil {
			return err
		}
		if err := os.Remove(filepath.Join(bin, "dot")); err != nil && !os.IsNotExist(err) {
			return err
		}
		if _, err := os.Stat(bin); err == nil {
			if err := syncDir(bin); err != nil {
				return err
			}
		}
	}
	if err := os.Remove(p); err != nil {
		return err
	}
	return syncDir(root)
}
func Uninstall(root string, consent bool) error {
	if !consent {
		return fmt.Errorf("explicit --yes uninstall consent required; private source/state/tracker are always retained")
	}
	unlock, err := lock(root)
	if err != nil {
		return err
	}
	defer unlock()
	if err := pending(root); err != nil {
		return err
	}
	if _, err := current(root); err != nil {
		return err
	}
	bin, err := native.BoundedPath(root, "bin")
	if err != nil {
		return err
	}
	p := filepath.Join(bin, "dot")
	v, err := os.Readlink(p)
	if err == nil {
		if v != "../current/dot" {
			return fmt.Errorf("foreign launch link")
		}
		if err := os.Remove(p); err != nil {
			return err
		}
		if err := syncDir(bin); err != nil {
			return err
		}
	} else if !os.IsNotExist(err) {
		return fmt.Errorf("foreign launch file")
	}
	if err := os.Remove(filepath.Join(root, "current")); err != nil && !os.IsNotExist(err) {
		return err
	}
	return syncDir(root)
}
