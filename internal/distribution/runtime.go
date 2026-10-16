package distribution

import (
	"context"
	"fmt"
	"github.com/ujjwal6792/dots-cli/internal/native"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
)

type Tools struct{ Git, Chezmoi string }

func Platform() string { return runtime.GOOS + "-" + runtime.GOARCH }

// Resolve never consults cwd, repository files or a private source checkout.
// Development binaries use a version-checked system dependency. Release binaries
// require their adjacent verified bundle, even when a system chezmoi exists.
func Resolve(ctx context.Context, version, git, chezmoi string) (Tools, error) {
	t := Tools{Git: git, Chezmoi: chezmoi}
	if t.Git == "" {
		t.Git = "git"
	}
	if p, err := exec.LookPath(t.Git); err == nil {
		t.Git = p
	}
	if version != "development" {
		exe, err := os.Executable()
		if err != nil {
			return t, err
		}
		exe, err = filepath.EvalSymlinks(exe)
		if err != nil {
			return t, err
		}
		a, err := ReadBundle(filepath.Dir(exe))
		if err != nil {
			return t, fmt.Errorf("release bundle invalid; reinstall a verified native archive (no system chezmoi fallback): %w", err)
		}
		if a.Version != version || a.Platform != Platform() {
			return t, fmt.Errorf("release metadata version/platform mismatch")
		}
		bundled := filepath.Join(filepath.Dir(exe), "libexec/chezmoi")
		if chezmoi != "" && chezmoi != bundled {
			return t, fmt.Errorf("release uses pinned bundled chezmoi; --chezmoi cannot substitute a system executable")
		}
		t.Chezmoi = bundled
	} else {
		if t.Chezmoi == "" {
			t.Chezmoi = "chezmoi"
		}
		if p, err := exec.LookPath(t.Chezmoi); err == nil {
			t.Chezmoi = p
		}
	}
	// Missing tools remain visible to setup's explicit dependency table; an
	// incompatible existing dependency must never schedule a manager overwrite.
	return t, nil
}
func CheckTool(ctx context.Context, path, name string) error {
	p, err := exec.LookPath(path)
	if err != nil {
		return fmt.Errorf("%s missing; use setup dependency review or install from the official repository", name)
	}
	b, err := exec.CommandContext(ctx, p, "--version").Output()
	if err != nil {
		return fmt.Errorf("%s incompatible executable", name)
	}
	v := strings.TrimSpace(string(b))
	if name == "git" && strings.HasPrefix(v, "git version ") {
		return nil
	}
	if name == "chezmoi" && (strings.HasPrefix(v, "chezmoi version v"+ChezmoiVersion+",") || strings.HasPrefix(v, "chezmoi version "+ChezmoiVersion+",")) {
		return nil
	}
	return fmt.Errorf("%s incompatible version; chezmoi must be pinned v%s, existing tools are never overwritten", name, ChezmoiVersion)
}

// ToolVersion reports the installed version for setup's dependency table:
// "2.49.0" for Git, "v2.73.1" for chezmoi. Empty when the executable is
// absent or its output is unrecognized. Read-only; safe during inspection.
func ToolVersion(ctx context.Context, path, name string) string {
	p, err := exec.LookPath(path)
	if err != nil {
		return ""
	}
	b, err := exec.CommandContext(ctx, p, "--version").Output()
	if err != nil {
		return ""
	}
	v := strings.TrimSpace(string(b))
	switch name {
	case "git":
		v = strings.TrimPrefix(v, "git version ")
	case "chezmoi":
		v = strings.TrimPrefix(v, "chezmoi version ")
	default:
		return ""
	}
	if v == "" {
		return ""
	}
	return strings.TrimSuffix(strings.Fields(v)[0], ",")
}
func ReadBundle(root string) (Manifest, error) {
	var m Manifest
	if !filepath.IsAbs(root) || filepath.Clean(root) != root {
		return m, fmt.Errorf("absolute clean bundle path required")
	}
	files := map[string][]byte{}
	for name, mode := range modes {
		p, err := native.BoundedPath(root, name)
		if err != nil {
			return m, err
		}
		st, err := os.Lstat(p)
		if err != nil {
			return m, err
		}
		if !st.Mode().IsRegular() || st.Mode().Perm() != os.FileMode(mode) {
			return m, fmt.Errorf("release file mode/type mismatch: %s", name)
		}
		if st.Size() > 160<<20 {
			return m, fmt.Errorf("release file too large")
		}
		b, err := os.ReadFile(p)
		if err != nil {
			return m, err
		}
		files[name] = b
	}
	if err := native.Decode(files["release.json"], &m); err != nil {
		return m, err
	}
	return m, validate(m, files)
}
