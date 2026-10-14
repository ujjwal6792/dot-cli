package main

import (
	"context"
	"flag"
	"fmt"
	"github.com/ujjwal6792/dots-cli/internal/acceptance"
	"os"
	"time"
)

func main() {
	archive := flag.String("archive", "", "real host-platform release archive")
	sha := flag.String("sha256", "", "independently verified archive checksum")
	dir := flag.String("dir", "", "new absolute fixture directory; retained for inspection")
	updateArchive := flag.String("update-archive", "", "optional DIFFERENT host-version archive for upgrade/rollback acceptance")
	updateSHA := flag.String("update-sha256", "", "trusted upgrade archive checksum")
	flag.Parse()
	if *archive == "" || *sha == "" || *dir == "" || (*updateArchive == "") != (*updateSHA == "") {
		fmt.Fprintln(os.Stderr, "--archive, --sha256 and --dir are required; update archive/checksum must be supplied together")
		os.Exit(2)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()
	if flag.NArg() != 0 {
		fmt.Fprintln(os.Stderr, "unexpected arguments")
		os.Exit(2)
	}
	if err := acceptance.Run(ctx, *archive, *sha, *dir); err != nil {
		fmt.Fprintln(os.Stderr, "dot-accept:", err)
		os.Exit(1)
	}
	if *updateArchive != "" || *updateSHA != "" {
		if err := acceptance.Upgrade(ctx, *archive, *sha, *updateArchive, *updateSHA, *dir); err != nil {
			fmt.Fprintln(os.Stderr, "dot-accept:", err)
			os.Exit(1)
		}
	}
}
