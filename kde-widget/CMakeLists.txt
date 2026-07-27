cmake_minimum_required(VERSION 3.16)
project(legioncontrol-widget)

find_package(ECM REQUIRED NO_MODULE)
set(CMAKE_MODULE_PATH ${ECM_MODULE_PATH})

find_package(KF6 REQUIRED COMPONENTS Package)

install(DIRECTORY package/
    DESTINATION ${KDE_INSTALL_DATADIR}/plasma/plasmoids/com.github.encomjp.legioncontrol
)
