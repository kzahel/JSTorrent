plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.serialization)
}

android {
    namespace = "com.jstorrent.companion"
    compileSdk = 36

    defaultConfig {
        minSdk = 26
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        consumerProguardFiles("consumer-rules.pro")
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }
    kotlinOptions {
        jvmTarget = "11"
    }
    testOptions {
        unitTests.isReturnDefaultValues = true
    }
    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
            excludes += "/META-INF/INDEX.LIST"
            excludes += "/META-INF/io.netty.versions.properties"
        }
    }
}

dependencies {
    // Depend on io-core
    implementation(project(":io-core"))

    // Pure Netty HTTP server (replaces Ktor for 6-10x better throughput)
    implementation("io.netty:netty-codec-http:4.1.100.Final")
    implementation("io.netty:netty-handler:4.1.100.Final")
    implementation("io.netty:netty-transport:4.1.100.Final")

    // Java-WebSocket for high-throughput /io and /control endpoints
    // This library achieves 8x better throughput than Ktor WebSocket
    implementation(libs.java.websocket)

    // Coroutines
    implementation(libs.kotlinx.coroutines.android)

    // JSON serialization
    implementation(libs.kotlinx.serialization.json)

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    testImplementation(libs.kotlin.test)
    testImplementation("org.mockito.kotlin:mockito-kotlin:5.2.1")
}
