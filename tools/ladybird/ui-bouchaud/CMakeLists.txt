# Bouchaud OS -- UI/Bouchaud : le frontend natif de Ladybird pour le
# gestionnaire de fenetres de Bouchaud (BOUCHAUD_UI_V1).
#
# Le binaire garde le nom que le noyau lance : BouchaudBrowserHost.

add_executable(BouchaudBrowserHost
    main.cpp
    Application.cpp
    BouchaudWebView.cpp
    BrowserWindow.cpp
)

target_include_directories(BouchaudBrowserHost PRIVATE
    ${LADYBIRD_SOURCE_DIR}
    ${LADYBIRD_SOURCE_DIR}/Services
)

target_link_libraries(BouchaudBrowserHost PRIVATE
    AK
    LibCore
    LibFileSystem
    LibGfx
    LibImageDecoderClient
    LibIPC
    LibJS
    LibMain
    LibRequests
    LibSync
    LibURL
    LibWakeLock
    LibWeb
    LibWebView
    OpenSSL::Crypto
    OpenSSL::SSL
    # Le chrome rend son texte avec Skia, le rasteriseur de la page. LibGfx le
    # lie en PRIVATE : ses en-tetes ne se propagent pas, il faut le nommer ici.
    skia
)

if (BOUCHAUD_PORT)
    target_link_options(BouchaudBrowserHost PRIVATE
        -static-pie
        LINKER:--allow-multiple-definition
    )
    set_target_properties(BouchaudBrowserHost PROPERTIES
        SKIP_BUILD_RPATH TRUE
        BUILD_WITH_INSTALL_RPATH FALSE
        INSTALL_RPATH ""
    )
endif()
