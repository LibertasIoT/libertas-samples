use std::env;
use std::path::{Path, PathBuf};

fn main() {
    let target = env::var("TARGET").expect("Cargo did not provide TARGET");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("Cargo did not provide OUT_DIR"));
    let xgboost_source = Path::new("xgboost")
        .canonicalize()
        .expect("vendored XGBoost source is missing");

    let bindings = bindgen::Builder::default()
        .header(
            xgboost_source
                .join("include")
                .join("xgboost")
                .join("c_api.h")
                .to_string_lossy(),
        )
        .clang_arg(format!("-I{}", xgboost_source.join("include").display()))
        .clang_arg(format!(
            "-I{}",
            xgboost_source.join("dmlc-core").join("include").display()
        ))
        .generate()
        .expect("unable to generate XGBoost C API bindings");
    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("unable to write XGBoost C API bindings");

    let destination = cmake::Config::new(&xgboost_source)
        .define("BUILD_STATIC_LIB", "ON")
        .define("USE_OPENMP", "OFF")
        .define("USE_CUDA", "OFF")
        .define("GOOGLE_TEST", "OFF")
        .define("ADD_PKGCONFIG", "OFF")
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        .define("CMAKE_BUILD_TYPE", "Release")
        .build();

    println!(
        "cargo:rustc-link-search=native={}",
        destination.join("lib").display()
    );
    println!(
        "cargo:rustc-link-search=native={}",
        destination.join("lib64").display()
    );
    println!("cargo:rustc-link-lib=static=xgboost");
    println!("cargo:rustc-link-lib=static=dmlc");

    if target.contains("apple") {
        println!("cargo:rustc-link-lib=dylib=c++");
    } else if target.contains("linux") {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }

    println!("cargo:rerun-if-changed=xgboost/CMakeLists.txt");
    println!("cargo:rerun-if-changed=xgboost/include/xgboost/c_api.h");
}
