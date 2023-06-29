use std::{io, thread};
use std::ffi::OsString;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::time::Duration;

use console::style;
use fs2::FileExt;
use windows_service::Error;
use windows_service::service::{
    ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use switch::core::{Config, Switch};

use crate::{BaseArgs, Commands, config, i18n};
use crate::command::{command, CommandEnum};

pub mod service;
mod windows_admin_check;

pub const SERVICE_FLAG: &'static str = "start_switch_service_v1_";
pub const SERVICE_NAME: &'static str = "switch-service-v1";
pub const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

fn admin_check() -> bool {
    if !windows_admin_check::is_app_elevated() {
        println!(
            "{}",
            style(i18n::switch_use_root_print()).red()
        );
        true
    } else {
        false
    }
}

fn not_started() -> bool {
    match service_state() {
        Ok(state) => {
            if state == ServiceState::Running {
                return false;
            } else {
                println!("{}", i18n::switch_service_not_start_print())
            }
        }
        Err(e) => {
            println!("{:?}", e);
        }
    }
    return true;
}

pub fn main0(base_args: BaseArgs) {
    match base_args.command {
        Commands::Start(args) => {
            if admin_check() {
                return;
            }
            {
                // 允许应用通过防火墙
                let _udp = UdpSocket::bind("0.0.0.0:0").unwrap();
            }
            let start_config = if let Some(config_path) = &args.config {
                match config::read_config_file(config_path.into()) {
                    Ok(start_config) => {
                        start_config
                    }
                    Err(e) => {
                        println!("{}", style(&e).red());
                        log::error!("{:?}", e);
                        return;
                    }
                }
            } else {
                match config::default_config(args) {
                    Ok(start_config) => {
                        start_config
                    }
                    Err(e) => {
                        println!("{}", style(&e).red());
                        log::error!("{:?}", e);
                        return;
                    }
                }
            };
            match service_state() {
                Ok(state) => {
                    if state == ServiceState::Stopped {
                        match start() {
                            Ok(_) => {
                                //需要检查启动状态
                                thread::sleep(Duration::from_secs(2));
                                println!("{}", style(i18n::switch_start_successfully_print()).green());
                            }
                            Err(e) => {
                                log::error!("{:?}", e);
                                println!("{}:{}", style(i18n::switch_start_failed_print()).red(), e);
                            }
                        }
                    } else {
                        println!("{}", i18n::switch_service_not_stopped_print());
                    }
                }
                Err(e) => {
                    match e {
                        Error::Winapi(ref e) => {
                            if let Some(code) = e.raw_os_error() {
                                if code == 1060 {
                                    //指定的服务未安装。
                                    let config = Config::new(
                                        start_config.tap,
                                        start_config.token,
                                        start_config.device_id,
                                        start_config.name,
                                        start_config.server,
                                        start_config.nat_test_server,
                                        start_config.in_ips,
                                        start_config.out_ips,
                                        start_config.password,
                                        start_config.simulate_multicast,
                                    );
                                    let lock = match config::lock_file() {
                                        Ok(lock) => {
                                            lock
                                        }
                                        Err(e) => {
                                            log::error!("文件锁定失败:{:?}",e);
                                            println!("文件锁定失败:{:?}", e);
                                            return;
                                        }
                                    };
                                    if lock.try_lock_exclusive().is_err() {
                                        println!("{}", style(i18n::switch_repeated_start_print()).red());
                                        return;
                                    }
                                    tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap().block_on(async move {
                                        match Switch::start(config).await {
                                            Ok(switch) => {
                                                crate::console_listen(&switch);
                                            }
                                            Err(e) => {
                                                log::error!("{:?}", e);
                                                println!("启动switch失败:{:?}", e);
                                            }
                                        }
                                    });
                                    lock.unlock().unwrap();
                                    return;
                                }
                            }
                        }
                        _ => {}
                    }
                    println!("{:?}", e);
                }
            };
            pause();
        }
        Commands::Stop => {
            if not_started() {
                return;
            }
            if admin_check() {
                return;
            }
            match stop() {
                Ok(_) => {
                    println!("{}", style(i18n::switch_stopped_print()).green())
                }
                Err(e) => {
                    log::error!("{:?}", e);
                    println!("停止失败:{}", e);
                }
            }
            pause();
        }
        Commands::Install(args) => {
            if admin_check() {
                return;
            }
            if service_state().is_ok() {
                println!("{}", i18n::switch_server_already_installed_print());
                return;
            }
            let path: PathBuf = args.path.into();
            if !path.exists() {
                std::fs::create_dir_all(&path).unwrap();
            }
            if !path.is_dir() {
                println!("参数必须为文件目录(Parameter must be a file directory)");
            } else {
                if let Err(e) = install(path, args.auto) {
                    log::error!("{:?}", e);
                    println!("安装失败:{}", e);
                } else {
                    println!("{}", style("安装成功(Installation succeeded)").green())
                }
            }
            pause();
        }
        Commands::Uninstall => {
            if admin_check() {
                return;
            }
            if service_state().is_err() {
                println!("服务未安装");
            }
            if let Err(e) = uninstall() {
                log::error!("{:?}", e);
                println!("卸载失败:{}", e);
            } else {
                println!("{}", style("卸载成功(Uninstall succeeded)").green())
            }
            pause();
        }
        Commands::Config(args) => {
            if service_state().is_err() {
                println!("服务未安装");
            }
            if let Err(e) = change(args.auto) {
                log::error!("{:?}", e);
                println!("配置失败:{}", e);
            } else {
                println!("{}", style("配置成功(Config succeeded)").green())
            }
            pause();
        }
        Commands::Route => {
            if not_started() {
                return;
            }
            command(CommandEnum::Route);
        }
        Commands::List { all } => {
            if not_started() {
                return;
            }
            if all {
                command(CommandEnum::ListAll);
            } else {
                command(CommandEnum::List);
            }
        }
        Commands::Status => {
            if not_started() {
                return;
            }
            command(CommandEnum::Status);
        }
    }
}

fn pause() {
    println!(
        "{}",
        style(i18n::switch_press_any_key_to_exit()).green()
    );
    use console::Term;
    let term = Term::stdout();
    let _ = term.read_char().unwrap();
}

fn install(mut path: PathBuf, auto: bool) -> Result<(), Error> {
    if !path.is_absolute() {
        path = path.canonicalize().unwrap();
    }
    let manager_access = ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;
    let current_exe_path = std::env::current_exe().unwrap();
    let service_path = path.join("switch-service-v1.exe");
    std::fs::copy(current_exe_path, service_path.as_path()).unwrap();
    if let Err(e) = std::fs::copy("wintun.dll", path.join("wintun.dll").as_path()) {
        if e.kind() == io::ErrorKind::NotFound {
            println!("'wintun.dll' not found. Please put 'wintun.dll' in the current directory");
            std::process::exit(0);
        } else {
            panic!("{:?}", e)
        }
    }
    let mut launch_arguments = Vec::new();
    launch_arguments.push(OsString::from(SERVICE_FLAG));
    launch_arguments.push(OsString::from(
        config::get_home().to_str().unwrap(),
    ));
    let start_type = if auto {
        ServiceStartType::AutoStart
    } else {
        ServiceStartType::OnDemand
    };
    let service_info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from("switch service v1"),
        service_type: SERVICE_TYPE,
        start_type,
        error_control: ServiceErrorControl::Normal,
        executable_path: service_path.into(),
        launch_arguments,
        dependencies: vec![],
        account_name: None, // run as System
        account_password: None,
    };
    let service = service_manager.create_service(&service_info, ServiceAccess::CHANGE_CONFIG)?;
    service.set_description("A VPN")?;
    Ok(())
}

fn change(auto: bool) -> Result<(), Error> {
    let manager_access = ServiceManagerAccess::CONNECT;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

    let service_access = ServiceAccess::QUERY_CONFIG | ServiceAccess::CHANGE_CONFIG;
    let service = service_manager.open_service(SERVICE_NAME, service_access)?;
    let config = service.query_config()?;
    let start_type = if auto {
        ServiceStartType::AutoStart
    } else {
        ServiceStartType::OnDemand
    };
    let executable_path = config.executable_path.to_string_lossy().to_string();
    let executable_path = if executable_path.starts_with('"') && executable_path.ends_with('"') {
        &executable_path[1..executable_path.len() - 1]
    } else {
        &executable_path
    };
    let mut split = executable_path.split(SERVICE_FLAG);
    let executable_path = split.next().unwrap().trim();
    let executable_path = if executable_path.starts_with('"') && executable_path.ends_with('"') {
        PathBuf::from(&executable_path[1..executable_path.len() - 1])
    } else {
        PathBuf::from(executable_path)
    };
    let home_path = split.next().unwrap().trim();
    let launch_arguments = vec![OsString::from(SERVICE_FLAG), OsString::from(home_path)];
    let service_info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: config.display_name,
        service_type: SERVICE_TYPE,
        start_type,
        error_control: config.error_control,
        executable_path,
        launch_arguments,
        dependencies: config.dependencies,
        account_name: None, // run as System
        account_password: None,
    };
    service.change_config(&service_info)?;
    Ok(())
}

fn uninstall() -> Result<(), Error> {
    let manager_access = ServiceManagerAccess::CONNECT;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

    let service_access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE;
    let service = service_manager.open_service(SERVICE_NAME, service_access)?;

    let service_status = service.query_status()?;
    if service_status.current_state != ServiceState::Stopped {
        service.stop()?;
        // Wait for service to stop
        thread::sleep(Duration::from_secs(1));
    }
    service.delete()?;
    Ok(())
}

fn start() -> Result<(), Error> {
    let manager_access = ServiceManagerAccess::CONNECT;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;
    let service = service_manager.open_service(SERVICE_NAME, ServiceAccess::START)?;
    let args: Vec<_> = std::env::args().collect();
    service.start(&args[1..])
}

fn service_state() -> Result<ServiceState, Error> {
    let manager_access = ServiceManagerAccess::CONNECT;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;

    let service_access = ServiceAccess::QUERY_STATUS;
    let service = service_manager.open_service(SERVICE_NAME, service_access)?;
    let service_status = service.query_status()?;
    return Ok(service_status.current_state);
}

fn stop() -> Result<(), Error> {
    let manager_access = ServiceManagerAccess::CONNECT;
    let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;
    let service = service_manager.open_service(SERVICE_NAME, ServiceAccess::STOP)?;
    service.stop()?;
    Ok(())
}
