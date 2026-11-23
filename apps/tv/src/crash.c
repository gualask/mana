/* Sends stderr to the app log and logs fatal signals with a backtrace, then
 * lets the signal kill the process as usual. An app started by the system
 * has no stderr we can read, and libraries report fatal errors there (an
 * FFmpeg assertion, an uncaught C++ exception in the media pipeline, which
 * runs inside the process). */
#include <execinfo.h>
#include <signal.h>
#include <string.h>
#include <unistd.h>

#define MAX_FRAMES 64

static int log_fd = -1;
static volatile sig_atomic_t handling;

static void on_fatal(int signal_number) {
    if (handling++) _exit(128 + signal_number); /* fault while reporting */
    char line[48] = "fatal signal ";
    int length = (int)strlen(line);
    if (signal_number >= 10) line[length++] = (char)('0' + signal_number / 10);
    line[length++] = (char)('0' + signal_number % 10);
    line[length++] = '\n';
    if (write(log_fd, line, length) < 0) {}
    void *frames[MAX_FRAMES];
    backtrace_symbols_fd(frames, backtrace(frames, MAX_FRAMES), log_fd);
    signal(signal_number, SIG_DFL);
    raise(signal_number);
}

void mana_install_crash_handler(int fd) {
    static const int fatal[] = {SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGABRT};
    log_fd = fd;
    dup2(fd, STDERR_FILENO);
    for (unsigned i = 0; i < sizeof fatal / sizeof fatal[0]; i++) signal(fatal[i], on_fatal);
}
