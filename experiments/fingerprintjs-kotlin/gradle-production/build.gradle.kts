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
            kotlin.srcDir("../src")
        }
    }
}

tasks.withType<org.jetbrains.kotlin.gradle.tasks.Kotlin2JsCompile>().configureEach {
    compilerOptions {
        target.set("es2015")
    }
}
