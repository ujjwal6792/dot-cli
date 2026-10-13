package main

import (
	"flag"
	"fmt"
	"github.com/ujjwal6792/dots-cli/internal/distribution"
	"io"
)

func installation(verb string, args []string, out, errOut io.Writer) error {
	fs := flag.NewFlagSet(verb, flag.ContinueOnError)
	fs.SetOutput(errOut)
	root := fs.String("root", "", "private installation root")
	archive := fs.String("archive", "", "prebuilt release archive")
	checksum := fs.String("sha256", "", "independently trusted checksum")
	yes := fs.Bool("yes", false, "explicit approval")
	if err := fs.Parse(args); err != nil {
		return usageError(err.Error())
	}
	if fs.NArg() != 0 || *root == "" {
		return usageError("--root ABSOLUTE_ROOT required; Homebrew/pacman-owned dot must use its original manager")
	}
	var err error
	switch verb {
	case "install", "self-update":
		if *archive == "" || *checksum == "" {
			return usageError("--archive and --sha256 required")
		}
		err = distribution.Install(*root, *archive, *checksum, verb == "self-update", *yes)
	case "uninstall":
		if *archive != "" || *checksum != "" {
			return usageError("uninstall does not accept an archive")
		}
		err = distribution.Uninstall(*root, *yes)
	case "install-recover":
		if *archive != "" || *checksum != "" {
			return usageError("recovery does not accept an archive")
		}
		err = distribution.RecoverInstall(*root, *yes)
	}
	if err == nil {
		fmt.Fprintln(out, verb+" complete; private source, tracker and machine state unchanged")
	}
	return err
}
