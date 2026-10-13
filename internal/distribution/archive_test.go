package distribution_test

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"github.com/ujjwal6792/dots-cli/internal/distribution"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestArchiveChecksumCheckedBeforeParsing(t *testing.T) {
	file := filepath.Join(t.TempDir(), "bad.tar.gz")
	os.WriteFile(file, []byte("not an archive"), 0600)
	if _, err := distribution.Verify(file, strings.Repeat("0", 64)); err == nil || !strings.Contains(err.Error(), "checksum") {
		t.Fatalf("%v", err)
	}
}

func TestVerifiedChecksumCannotAuthorizeLinksTraversalOrDuplicateInventory(t *testing.T) {
	for _, headers := range [][]*tar.Header{
		{{Name: "dot", Typeflag: tar.TypeSymlink, Linkname: "/bin/sh", Mode: 0755}},
		{{Name: "../dot", Typeflag: tar.TypeReg, Mode: 0755}},
		{{Name: "dot", Typeflag: tar.TypeReg, Mode: 0755}, {Name: "dot", Typeflag: tar.TypeReg, Mode: 0755}},
	} {
		var b bytes.Buffer
		z := gzip.NewWriter(&b)
		tw := tar.NewWriter(z)
		for _, h := range headers {
			if err := tw.WriteHeader(h); err != nil {
				t.Fatal(err)
			}
		}
		tw.Close()
		z.Close()
		file := filepath.Join(t.TempDir(), "hostile.tar.gz")
		os.WriteFile(file, b.Bytes(), 0600)
		if _, err := distribution.Verify(file, distribution.Hash(b.Bytes())); err == nil {
			t.Fatalf("accepted %+v", headers)
		}
	}
}

func TestReleaseVersionIsStrictSemanticVersion(t *testing.T) {
	for _, v := range []string{"v1.2.3", "1.2.3-..", "1.2.3-01", "1.2.3+", "1.2.3+build..x", "01.2.3", "1.2.3/a"} {
		if distribution.ValidVersion(v) {
			t.Fatalf("accepted %q", v)
		}
	}
	for _, v := range []string{"0.10.18-rc.1", "1.2.3", "1.2.3-rc.0+build.01"} {
		if !distribution.ValidVersion(v) {
			t.Fatalf("rejected %q", v)
		}
	}
}
