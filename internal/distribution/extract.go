package distribution

import (
	"fmt"
	"os"
	"path/filepath"
)

// Extract publishes to an exclusively created directory only. Archive objects
// are revalidated: callers cannot mutate verified metadata into traversal paths.
func Extract(a *Archive, dir string) error {
	if a == nil {
		return fmt.Errorf("verified archive required")
	}
	if len(a.Files) != 4 {
		return fmt.Errorf("unexpected archive inventory")
	}
	if err := validate(a.Manifest, a.Files); err != nil {
		return err
	}
	if err := safeRoot(dir); err != nil {
		return err
	}
	if err := os.Mkdir(dir, 0700); err != nil {
		return err
	}
	for _, name := range []string{"dot", "libexec/chezmoi", "licenses/chezmoi.txt", "release.json"} {
		p := filepath.Join(dir, name)
		if err := os.MkdirAll(filepath.Dir(p), 0755); err != nil {
			return err
		}
		if err := writeExclusive(p, a.Files[name], os.FileMode(modes[name])); err != nil {
			return err
		}
	}
	_, err := ReadBundle(dir)
	return err
}
