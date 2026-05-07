SHELL := /bin/sh

# Copr's "custom" source method invokes this target inside a clean buildroot
# to produce a buildable SRPM. `rpkg` consumes the spec next to this Makefile
# (../proteus.spec) plus the project sources from the cloned git repo, then
# emits the SRPM into $(outdir), which Copr then picks up and dispatches to
# the per-chroot builders.
#
# Reference: https://docs.pagure.org/copr.copr/user_documentation.html
srpm:
	dnf -y install rpkg
	rpkg srpm --outdir $(outdir)
