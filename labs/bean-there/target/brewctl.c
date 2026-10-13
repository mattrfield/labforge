// Staff helper: prints the espresso machine status. Installed SUID root on medium.
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

int main(void) {
    if (setresgid(0, 0, 0) != 0 || setresuid(0, 0, 0) != 0) {
        fprintf(stderr, "brewctl: must be installed setuid root\n");
    }
    puts("== Bean There brew control ==");
    // Relative command name: resolved through the caller's PATH.
    return system("date '+Checked at %H:%M' && cat /etc/brewctl/status");
}
