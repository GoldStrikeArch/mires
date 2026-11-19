// Shared Gradle project for every Kotlin/JS port: the production lane of
// experiments/fingerprintjs-kotlin/gradle-production (Kotlin Gradle plugin 2.4.10, whole-program
// ES module output, ES2015). languages/kotlin/build.mjs copies this template into the port's work
// directory and substitutes the port's source directory and dependencies.
plugins {
    kotlin("multiplatform") version "2.4.10"
}

repositories {
    mavenCentral()
}

kotlin {
    js {
        browser()
        binaries.executable()
        useEsModules()
    }
    sourceSets {
        val jsMain by getting {
            kotlin.srcDir("@@SOURCES@@")
            dependencies {
@@DEPENDENCIES@@
            }
        }
    }
}

tasks.withType<org.jetbrains.kotlin.gradle.tasks.Kotlin2JsCompile>().configureEach {
    compilerOptions {
        target.set("es2015")
    }
}
