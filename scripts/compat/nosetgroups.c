/* For the chroot SFTP test of scripts/compat/servers.sh only: sshd runs as
 * root of a user namespace, where the kernel refuses setgroups() (only one
 * user is mapped, without root). It is loaded into that sshd alone and lets
 * the call succeed with the groups unchanged; the chroot itself is real. */
#include <grp.h>
#include <sys/types.h>
int setgroups(size_t size, const gid_t *list) { (void)size; (void)list; return 0; }
int initgroups(const char *user, gid_t group) { (void)user; (void)group; return 0; }
