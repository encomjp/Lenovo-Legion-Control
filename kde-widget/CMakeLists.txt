cmake_minimum_required(VERSION 3.16)
project(legioncontrol-widget)

find_package(ECM REQUIRED NO_MODULE)
set(CMAKE_MODULE_PATH ${ECM_MODULE_PATH})

include(KDEInstallDirs)
include(KDECMakeSettings)

find_package(KF6 REQUIRED COMPONENTS Package)

kpackage_install_package(package com.github.encomjp.legioncontrol plasmoids)
