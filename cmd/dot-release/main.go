// dot-release is maintainer-only Go tooling, never a runtime dependency.
package main

import (
	"context"
	"flag"
	"fmt"
	"github.com/ujjwal6792/dots-cli/internal/distribution"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

func main() {
	if err := run(os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, "dot-release:", err)
		os.Exit(1)
	}
}
func run(args []string) error {
	if len(args) == 0 {
		return fmt.Errorf("usage: dot-release build --version VERSION [--out dist] [--deps-dir DIR] | verify --archive FILE --sha256 HASH | extract --archive FILE --sha256 HASH --dir NEW_DIR")
	}
	verb := args[0]
	fs := flag.NewFlagSet(verb, flag.ContinueOnError)
	version := fs.String("version", "", "release version without v")
	out := fs.String("out", "dist", "output directory")
	deps := fs.String("deps-dir", "", "verified upstream archive cache (otherwise downloads pinned assets)")
	archive := fs.String("archive", "", "archive path")
	checksum := fs.String("sha256", "", "trusted archive hash")
	dir := fs.String("dir", "", "new extraction directory")
	if err := fs.Parse(args[1:]); err != nil {
		return err
	}
	if fs.NArg() != 0 {
		return fmt.Errorf("unexpected arguments")
	}
	allowed := map[string]bool{}
	switch verb {
	case "build":
		allowed = map[string]bool{"version": true, "out": true, "deps-dir": true}
	case "verify":
		allowed = map[string]bool{"archive": true, "sha256": true, "version": true}
	case "extract":
		allowed = map[string]bool{"archive": true, "sha256": true, "version": true, "dir": true}
	}
	var invalid string
	fs.Visit(func(f *flag.Flag) {
		if !allowed[f.Name] {
			invalid = f.Name
		}
	})
	if invalid != "" {
		return fmt.Errorf("unsupported --%s for %s", invalid, verb)
	}
	switch verb {
	case "verify", "extract":
		a, err := distribution.Verify(*archive, *checksum)
		if err != nil {
			return err
		}
		if *version != "" && a.Manifest.Version != *version {
			return fmt.Errorf("unexpected release version")
		}
		if verb == "extract" {
			if *dir == "" {
				return fmt.Errorf("--dir required")
			}
			if err := distribution.Extract(a, *dir); err != nil {
				return err
			}
		}
		fmt.Printf("verified %s %s %s (binary formats checked; execution not implied)\n", a.Manifest.Version, a.Manifest.Platform, a.SHA256)
		return nil
	case "build":
		if !distribution.ValidVersion(*version) {
			return fmt.Errorf("valid semantic --version required")
		}
		if err := os.MkdirAll(*out, 0755); err != nil {
			return err
		}
		stage, err := os.MkdirTemp(*out, ".build-")
		if err != nil {
			return err
		}
		defer os.Remove(stage)
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Minute)
		defer cancel()
		checksumsAsset := "chezmoi_" + distribution.ChezmoiVersion + "_checksums.txt"
		var official []byte
		if *deps != "" {
			official, err = os.ReadFile(filepath.Join(*deps, checksumsAsset))
		} else {
			official, err = download(ctx, "https://github.com/twpayne/chezmoi/releases/download/v"+distribution.ChezmoiVersion+"/"+checksumsAsset)
		}
		if err != nil {
			return err
		}
		if err := distribution.VerifyUpstreamChecksums(official); err != nil {
			return err
		}
		var sums strings.Builder
		for _, platform := range distribution.Platforms() {
			pin, _ := distribution.Pinned(platform)
			asset := "chezmoi_" + distribution.ChezmoiVersion + "_" + strings.ReplaceAll(platform, "-", "_") + ".tar.gz"
			var b []byte
			if *deps != "" {
				b, err = os.ReadFile(filepath.Join(*deps, asset))
			} else {
				b, err = download(ctx, "https://github.com/twpayne/chezmoi/releases/download/v"+distribution.ChezmoiVersion+"/"+asset)
			}
			if err != nil {
				return err
			}
			if distribution.Hash(b) != pin.Archive {
				return fmt.Errorf("dependency checksum mismatch: %s", asset)
			}
			chezmoi, license, err := distribution.Dependency(b, platform)
			if err != nil {
				return err
			}
			target := strings.Split(platform, "-")
			dotFile := filepath.Join(stage, "dot")
			c := exec.CommandContext(ctx, "go", "build", "-trimpath", "-buildvcs=false", "-ldflags", "-s -w -X main.version="+*version+" -X main.buildIdentity="+distribution.Identity(*version), "-o", dotFile, "./cmd/dot")
			c.Env = append(os.Environ(), "CGO_ENABLED=0", "GOWORK=off", "GOFLAGS=", "GOEXPERIMENT=", "GOAMD64=v1", "GOARM64=v8.0", "GOOS="+target[0], "GOARCH="+target[1])
			c.Stdout = os.Stdout
			c.Stderr = os.Stderr
			if err := c.Run(); err != nil {
				return err
			}
			dot, err := os.ReadFile(dotFile)
			if err != nil {
				return err
			}
			if err := os.Remove(dotFile); err != nil {
				return err
			}
			packed, err := distribution.Package(*version, platform, dot, chezmoi, license)
			if err != nil {
				return err
			}
			name := "dot-" + *version + "-" + platform + ".tar.gz"
			file := filepath.Join(*out, name)
			if err := os.WriteFile(file, packed, 0644); err != nil {
				return err
			}
			hash := distribution.Hash(packed)
			if _, err := distribution.Verify(file, hash); err != nil {
				return err
			}
			fmt.Fprintf(&sums, "%s  %s\n", hash, name)
			fmt.Printf("built and verified %s\n", file)
		}
		return os.WriteFile(filepath.Join(*out, "SHA256SUMS"), []byte(sums.String()), 0644)
	default:
		return fmt.Errorf("unknown release operation %q", verb)
	}
}
func download(ctx context.Context, url string) ([]byte, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return nil, err
	}
	client := &http.Client{Timeout: 2 * time.Minute, CheckRedirect: func(req *http.Request, via []*http.Request) error {
		if len(via) > 8 || req.URL.Scheme != "https" {
			return fmt.Errorf("unsafe dependency redirect")
		}
		return nil
	}}
	res, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer res.Body.Close()
	if res.StatusCode != 200 {
		return nil, fmt.Errorf("dependency download: HTTP %d", res.StatusCode)
	}
	return io.ReadAll(io.LimitReader(res.Body, 160<<20))
}
