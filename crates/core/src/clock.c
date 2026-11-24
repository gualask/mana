/* The local time, which Rust's standard library does not give: the
 * offset from UTC that localtime_r applies, as the system's time zone
 * says (on the TV, /etc/localtime). time_t is 32 bits on the TV's glibc
 * 2.28, so the conversion stays in C. */
#define _DEFAULT_SOURCE
#include <time.h>

long mana_utc_offset(long long seconds) {
    time_t at = (time_t)seconds;
    struct tm local;
    if (!localtime_r(&at, &local)) {
        return 0;
    }
    return local.tm_gmtoff;
}
