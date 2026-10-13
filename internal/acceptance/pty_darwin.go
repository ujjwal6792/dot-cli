package acceptance

import (
	"bytes"
	"fmt"
	"golang.org/x/sys/unix"
	"os"
	"unsafe"
)

func openPTY() (*os.File, *os.File, error) {
	fd, err := unix.Open("/dev/ptmx", unix.O_RDWR|unix.O_NOCTTY|unix.O_CLOEXEC, 0)
	if err != nil {
		return nil, nil, err
	}
	master := os.NewFile(uintptr(fd), "pty-master")
	fail := func(err error) (*os.File, *os.File, error) { master.Close(); return nil, nil, err }
	for _, request := range []uint{unix.TIOCPTYGRANT, unix.TIOCPTYUNLK} {
		if err := unix.IoctlSetInt(fd, request, 0); err != nil {
			return fail(err)
		}
	}
	var name [128]byte
	if _, _, errno := unix.Syscall(unix.SYS_IOCTL, uintptr(fd), unix.TIOCPTYGNAME, uintptr(unsafe.Pointer(&name[0]))); errno != 0 {
		return fail(errno)
	}
	n := bytes.IndexByte(name[:], 0)
	if n <= 0 {
		return fail(fmt.Errorf("invalid PTY name"))
	}
	slave, err := os.OpenFile(string(name[:n]), os.O_RDWR, 0)
	if err != nil {
		return fail(err)
	}
	if err := unix.IoctlSetWinsize(fd, unix.TIOCSWINSZ, &unix.Winsize{Row: 32, Col: 100}); err != nil {
		slave.Close()
		return fail(err)
	}
	return master, slave, nil
}
