// Package distribution owns the native release format and private installation.
// The checksum supplied by the caller is a trust input, not read from the archive.
package distribution

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"crypto/sha256"
	"debug/buildinfo"
	"debug/elf"
	"debug/macho"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"github.com/ujjwal6792/dots-cli/internal/native"
	"io"
	"os"
	"regexp"
	"sort"
	"strings"
	"time"
)

const ChezmoiVersion = "2.73.1"
const LicenseHash = "6ce2375d89522ac7a1417320c8406886bc0ecb984baf41e3c27b8a07a81a30b2"
const ChecksumsHash = "f5dc34c2c953795703492c83a91e460b62c7d17b919d6d01fdf18a23904897f6"

type Pin struct{ Archive, Binary string }

// Immutable upstream v2.73.1 release assets; independently checked against the
// official checksum file and downloaded bytes on 2026-10-14.
var pins = map[string]Pin{
	"darwin-arm64": {"19a8d9778d90476cc2f0463be69e9d82e2b91ba7b2361dbfc5348f243b1ba731", "48bb0988507562f6ee1dca129182d681cf5648ef55b80a2201049a23ed615980"},
	"linux-amd64":  {"a3f958acb7d0e5d53fa43c2eec9bc8ae3b4fba8bcbfdce29a8881ca3ffc20a53", "fbf312e45c5c1f629e1d994c201cdd666198bd556af38512f4432ae6d3952521"},
	"linux-arm64":  {"e60275b27f505ae2086bfb87b144eacf7377a89803120f6bd933d1852a93b0c6", "c8ca4eddd5777156be9aa64de63f24046866228b31ae28eb8b8509043a899ce3"},
}

func Pinned(platform string) (Pin, error) {
	p, ok := pins[platform]
	if !ok {
		return p, fmt.Errorf("unsupported platform %q", platform)
	}
	return p, nil
}
func Platforms() []string { return []string{"darwin-arm64", "linux-amd64", "linux-arm64"} }
func VerifyUpstreamChecksums(b []byte) error {
	if Hash(b) != ChecksumsHash {
		return fmt.Errorf("upstream checksum-file immutable hash mismatch")
	}
	for platform, p := range pins {
		asset := "chezmoi_" + ChezmoiVersion + "_" + strings.ReplaceAll(platform, "-", "_") + ".tar.gz"
		if !strings.Contains(string(b), p.Archive+"  "+asset+"\n") {
			return fmt.Errorf("upstream checksum entry missing: %s", asset)
		}
	}
	return nil
}
func Hash(b []byte) string { s := sha256.Sum256(b); return hex.EncodeToString(s[:]) }
func ValidVersion(v string) bool {
	if !regexp.MustCompile(`^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$`).MatchString(v) {
		return false
	}
	base, build, hasBuild := strings.Cut(v, "+")
	if hasBuild {
		for _, part := range strings.Split(build, ".") {
			if part == "" {
				return false
			}
		}
	}
	_, pre, hasPre := strings.Cut(base, "-")
	if hasPre {
		for _, part := range strings.Split(pre, ".") {
			if part == "" {
				return false
			}
			numeric := strings.IndexFunc(part, func(r rune) bool { return r < '0' || r > '9' }) < 0
			if numeric && len(part) > 1 && part[0] == '0' {
				return false
			}
		}
	}
	return true
}
func Identity(v string) string { return "dot-native-v1:" + v + ":end" }

type File struct {
	SHA256 string `json:"sha256"`
	Mode   int64  `json:"mode"`
}
type Manifest struct {
	Schema            int             `json:"schema"`
	Version           string          `json:"version"`
	Platform          string          `json:"platform"`
	Chezmoi           string          `json:"chezmoi"`
	DependencyRuntime string          `json:"dependency_runtime"`
	Files             map[string]File `json:"files"`
}
type Archive struct {
	Manifest Manifest
	Files    map[string][]byte
	SHA256   string
}

var modes = map[string]int64{"dot": 0755, "libexec/chezmoi": 0755, "licenses/chezmoi.txt": 0644, "release.json": 0644}

func Verify(path, expected string) (*Archive, error) {
	if !regexp.MustCompile(`^[a-f0-9]{64}$`).MatchString(expected) {
		return nil, fmt.Errorf("trusted lowercase SHA256 checksum required")
	}
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	b, err := io.ReadAll(io.LimitReader(f, 160<<20))
	if err != nil {
		return nil, err
	}
	if Hash(b) != expected {
		return nil, fmt.Errorf("archive checksum mismatch")
	}
	files, headers, err := readTar(b, 160<<20)
	if err != nil {
		return nil, err
	}
	if len(files) != len(modes) {
		return nil, fmt.Errorf("unexpected archive inventory")
	}
	for name, mode := range modes {
		if _, ok := files[name]; !ok || headers[name].Mode != mode {
			return nil, fmt.Errorf("missing file or unsafe mode: %s", name)
		}
	}
	var m Manifest
	if err := native.Decode(files["release.json"], &m); err != nil {
		return nil, fmt.Errorf("invalid release metadata: %w", err)
	}
	if err := validate(m, files); err != nil {
		return nil, err
	}
	return &Archive{m, files, expected}, nil
}
func validate(m Manifest, files map[string][]byte) error {
	p, err := Pinned(m.Platform)
	if err != nil {
		return err
	}
	if m.Schema != 1 || !ValidVersion(m.Version) || m.Chezmoi != ChezmoiVersion || len(m.Files) != 3 {
		return fmt.Errorf("release version/schema/inventory mismatch")
	}
	requirements, err := DependencyRuntime(files["libexec/chezmoi"], m.Platform)
	if err != nil {
		return err
	}
	if m.DependencyRuntime != requirements {
		return fmt.Errorf("dependency runtime metadata mismatch")
	}
	for _, name := range []string{"dot", "libexec/chezmoi", "licenses/chezmoi.txt"} {
		f, ok := m.Files[name]
		if !ok || f.Mode != modes[name] || f.SHA256 != Hash(files[name]) {
			return fmt.Errorf("release hash/mode mismatch: %s", name)
		}
	}
	if Hash(files["libexec/chezmoi"]) != p.Binary || Hash(files["licenses/chezmoi.txt"]) != LicenseHash {
		return fmt.Errorf("bundled dependency differs from immutable pin")
	}
	info, err := buildinfo.Read(bytes.NewReader(files["dot"]))
	if err != nil || info.Path != "github.com/ujjwal6792/dots-cli/cmd/dot" || !bytes.Contains(files["dot"], []byte(Identity(m.Version))) {
		return fmt.Errorf("dot executable version/product stamp differs from metadata")
	}
	dependency, err := buildinfo.Read(bytes.NewReader(files["libexec/chezmoi"]))
	if err != nil || dependency.Main.Path != "chezmoi.io/chezmoi/v2" || dependency.Main.Version != "v"+ChezmoiVersion {
		return fmt.Errorf("chezmoi executable version mismatch")
	}
	for _, name := range []string{"dot", "libexec/chezmoi"} {
		if err := CheckBinary(files[name], m.Platform); err != nil {
			return fmt.Errorf("%s: %w", name, err)
		}
	}
	return nil
}
func CheckBinary(b []byte, platform string) error {
	if platform == "darwin-arm64" {
		f, err := macho.NewFile(bytes.NewReader(b))
		if err != nil {
			return err
		}
		defer f.Close()
		if f.Cpu != macho.CpuArm64 || f.Type != macho.TypeExec {
			return fmt.Errorf("not Darwin ARM64 executable")
		}
		return nil
	}
	f, err := elf.NewFile(bytes.NewReader(b))
	if err != nil {
		return err
	}
	defer f.Close()
	want := elf.EM_AARCH64
	if platform == "linux-amd64" {
		want = elf.EM_X86_64
	}
	if f.Machine != want || f.Class != elf.ELFCLASS64 || (f.Type != elf.ET_EXEC && f.Type != elf.ET_DYN) {
		return fmt.Errorf("wrong Linux executable format")
	}
	return nil
}

// DependencyRuntime records actual ELF loader/shared-library requirements, not
// a promise that cross-builds work on every glibc/musl distribution.
func DependencyRuntime(b []byte, platform string) (string, error) {
	if platform == "darwin-arm64" {
		return "darwin-arm64", nil
	}
	f, err := elf.NewFile(bytes.NewReader(b))
	if err != nil {
		return "", err
	}
	defer f.Close()
	loader := "static"
	for _, p := range f.Progs {
		if p.Type == elf.PT_INTERP {
			v, err := io.ReadAll(io.LimitReader(p.Open(), 4096))
			if err != nil {
				return "", err
			}
			loader = strings.TrimRight(string(v), "\x00")
		}
	}
	libraries, err := f.ImportedLibraries()
	if err != nil {
		return "", err
	}
	sort.Strings(libraries)
	if len(libraries) > 0 {
		loader += "; libraries=" + strings.Join(libraries, ",")
	}
	return loader, nil
}
func readTar(b []byte, max int64) (map[string][]byte, map[string]*tar.Header, error) {
	compressed := bytes.NewReader(b)
	z, err := gzip.NewReader(compressed)
	if err != nil {
		return nil, nil, err
	}
	defer z.Close()
	z.Multistream(false)
	t := tar.NewReader(z)
	files := map[string][]byte{}
	headers := map[string]*tar.Header{}
	var total int64
	for {
		h, err := t.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			return nil, nil, err
		}
		if h.Typeflag != tar.TypeReg || h.Size < 0 || h.Size > max || total+h.Size > max || len(files) > 32 {
			return nil, nil, fmt.Errorf("archive contains unsupported file or size")
		}
		if _, ok := files[h.Name]; ok {
			return nil, nil, fmt.Errorf("duplicate archive path")
		}
		// Files are kept in memory; no untrusted path is ever extracted here.
		v, err := io.ReadAll(t)
		if err != nil {
			return nil, nil, err
		}
		files[h.Name] = v
		headers[h.Name] = h
		total += h.Size
	}
	// Consume gzip trailer: do not accept truncated or checksum-corrupt streams.
	trailing, err := io.ReadAll(io.LimitReader(z, (1<<20)+1))
	if err != nil {
		return nil, nil, err
	}
	if len(trailing) > 1<<20 || compressed.Len() != 0 {
		return nil, nil, fmt.Errorf("trailing archive stream")
	}
	for _, v := range trailing {
		if v != 0 {
			return nil, nil, fmt.Errorf("trailing non-padding archive bytes")
		}
	}
	return files, headers, nil
}
func Package(version, platform string, dot, chezmoi, license []byte) ([]byte, error) {
	files := map[string][]byte{"dot": dot, "libexec/chezmoi": chezmoi, "licenses/chezmoi.txt": license}
	m := Manifest{Schema: 1, Version: version, Platform: platform, Chezmoi: ChezmoiVersion, Files: map[string]File{}}
	var err error
	m.DependencyRuntime, err = DependencyRuntime(chezmoi, platform)
	if err != nil {
		return nil, err
	}
	for name, b := range files {
		m.Files[name] = File{Hash(b), modes[name]}
	}
	if err := validate(m, files); err != nil {
		return nil, err
	}
	meta, err := json.MarshalIndent(m, "", "  ")
	if err != nil {
		return nil, err
	}
	files["release.json"] = append(meta, '\n')
	var out bytes.Buffer
	z := gzip.NewWriter(&out)
	z.Header.OS = 255
	t := tar.NewWriter(z)
	names := make([]string, 0, len(files))
	for name := range files {
		names = append(names, name)
	}
	sort.Strings(names)
	for _, name := range names {
		b := files[name]
		if err := t.WriteHeader(&tar.Header{Name: name, Mode: modes[name], Size: int64(len(b)), Typeflag: tar.TypeReg, ModTime: time.Unix(0, 0), Format: tar.FormatUSTAR}); err != nil {
			return nil, err
		}
		if _, err := t.Write(b); err != nil {
			return nil, err
		}
	}
	if err := t.Close(); err != nil {
		return nil, err
	}
	if err := z.Close(); err != nil {
		return nil, err
	}
	return out.Bytes(), nil
}

// Dependency verifies upstream bytes before selecting the two permitted assets.
func Dependency(b []byte, platform string) ([]byte, []byte, error) {
	pin, err := Pinned(platform)
	if err != nil {
		return nil, nil, err
	}
	if Hash(b) != pin.Archive {
		return nil, nil, fmt.Errorf("upstream archive checksum mismatch")
	}
	files, headers, err := readTar(b, 160<<20)
	if err != nil {
		return nil, nil, err
	}
	if files["chezmoi"] == nil || headers["chezmoi"].Mode&0111 == 0 || Hash(files["chezmoi"]) != pin.Binary || Hash(files["LICENSE"]) != LicenseHash {
		return nil, nil, fmt.Errorf("upstream executable/license pin mismatch")
	}
	if err := CheckBinary(files["chezmoi"], platform); err != nil {
		return nil, nil, err
	}
	return files["chezmoi"], files["LICENSE"], nil
}
