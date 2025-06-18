Build Order

  Tier 1 - Foundation:
  1. m4
  2. make
  3. pkgconf
  4. zlib
  5. zstd
  6. brotli
  7. nghttp2
  8. openssl
  9. tar
  10. libidn2

  Tier 2 - Math libraries:
  11. gmp (needs m4)
  12. mpfr (needs gmp)
  13. mpc (needs gmp, mpfr)
  14. isl (needs gmp)

  Tier 3 - Toolchain:                                                                     15. binutils (needs zlib)
  16. gcc (needs gmp, mpfr, mpc, isl, zstd)

  Tier 4 - Network libraries:
  17. libssh2 (needs openssl, zlib)
  18. libpsl (needs libidn2)

  Tier 5 - Applications:
  19. curl (needs openssl, zlib, nghttp2, brotli, libssh2, libidn2, libpsl)

  Tier 6 - Rust packages (anytime):
  20. bat
  21. helix
  22. ripgrep
