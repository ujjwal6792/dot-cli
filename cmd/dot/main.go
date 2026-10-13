// Command dot is the single native CLI entrypoint. Runtime implementation is
// tracked in #16–#18; the retired interpreter-backed CLI is not a fallback.
package main

import (
	"fmt"
	"os"
)

var version = "development"

func main() {
	if len(os.Args) == 2 {
		switch os.Args[1] {
		case "--help", "-h", "help":
			fmt.Println("dot — native Go CLI (under construction)\nusage: dot --help | --version\n\nSetup, TUI, and configuration commands are pending issues #16–#18.")
			return
		case "--version", "-v":
			fmt.Println("dot " + version)
			return
		}
	}
	fmt.Fprintln(os.Stderr, "dot: native runtime is not implemented yet; see issues #16–#18")
	os.Exit(1)
}
