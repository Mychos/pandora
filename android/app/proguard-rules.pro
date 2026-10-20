# The only names R8 cannot see are the ones the native side asks for by hand.
#
# `shared/android.rs` reaches `dev.pandora.Pandora`'s methods through JNI, one
# signature per method: the lookup is a string, so nothing in the dex references
# them and R8 would rename them out from under the Rust half. `MainActivity` is
# in the manifest, so the framework's own keep rules already cover it, and the
# `native` methods on GameActivity are kept by that AAR's consumer rules.
#
# Renaming a method on either side of the JNI boundary means renaming it in both,
# whether or not this file is in the way — see `android/README.md`.
-keep class dev.pandora.Pandora { *; }
