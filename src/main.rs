use byteorder::{LittleEndian, ReadBytesExt};
use rusb::{Context, Device, DeviceHandle, Result, UsbContext};
use std::{io::Cursor, time::Duration};

const VID: u16 = 0x1b1c;
const PID: u16 = 0x0c22;

#[derive(Debug)]
struct Endpoint {
    config: u8,
    iface: u8,
    setting: u8,
    address: u8,
}

fn main() -> Result<()> {
    let mut context = Context::new()?;
    let (mut device, mut handle) =
        open_device(&mut context, VID, PID).expect("Did not find USB device");

    // print_device_info(&mut handle)?;

    let endpoints = find_readable_endpoints(&mut device)?;
    let endpoint = endpoints
        .first()
        .expect("No Configurable endpoint found on device");

    let has_kernel_driver = match handle.kernel_driver_active(endpoint.iface) {
        Ok(true) => {
            handle.detach_kernel_driver(endpoint.iface)?;
            true
        }
        _ => false,
    };
    // println!("has kernel driver? {}", has_kernel_driver);
    // control device here

    // claim and configure device
    configure_endpoint(&mut handle, &endpoint)?;

    set_idle(&mut handle).ok();
    set_report(&mut handle)?;
    let data = read_interrupt(&mut handle, endpoint.address)?;
    // println!("{:02X?}", &data);
    print_data(data);
    // cleanup after use
    handle.release_interface(endpoint.iface)?;
    if has_kernel_driver {
        handle.attach_kernel_driver(endpoint.iface)?;
    }
    Ok(())
}

fn open_device<T: UsbContext>(
    context: &mut T,
    vid: u16,
    pid: u16,
) -> Option<(Device<T>, DeviceHandle<T>)> {
    let devices = match context.devices() {
        Ok(d) => d,
        Err(_) => return None,
    };

    for device in devices.iter() {
        let device_desc = match device.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };

        if device_desc.vendor_id() == vid && device_desc.product_id() == pid {
            match device.open() {
                Ok(handle) => return Some((device, handle)),
                Err(_) => continue,
            }
        }
    }

    None
}

fn print_device_info<T: UsbContext>(handle: &mut DeviceHandle<T>) -> Result<()> {
    let device_desc = handle.device().device_descriptor()?;
    let timeout = Duration::from_secs(1);
    let languages = handle.read_languages(timeout)?;

    println!("Active configuration: {}", handle.active_configuration()?);

    if !languages.is_empty() {
        let language = languages[0];
        println!("Language: {:?}", language);

        println!(
            "Manufacturer: {}",
            handle
                .read_manufacturer_string(language, &device_desc, timeout)
                .unwrap_or_else(|_| "Not Found".to_string())
        );
        println!(
            "Product: {}",
            handle
                .read_product_string(language, &device_desc, timeout)
                .unwrap_or_else(|_| "Not Found".to_string())
        );
        println!(
            "Serial Number: {}",
            handle
                .read_serial_number_string(language, &device_desc, timeout)
                .unwrap_or_else(|_| "Not Found".to_string())
        );
    }
    Ok(())
}

// returns all readable endpoints for given usb device and descriptor
fn find_readable_endpoints<T: UsbContext>(device: &mut Device<T>) -> Result<Vec<Endpoint>> {
    let device_desc = device.device_descriptor()?;
    let mut endpoints = vec![];
    for n in 0..device_desc.num_configurations() {
        let config_desc = match device.config_descriptor(n) {
            Ok(c) => c,
            Err(_) => continue,
        };
        // println!("{:#?}", config_desc);
        for interface in config_desc.interfaces() {
            for interface_desc in interface.descriptors() {
                // println!("{:#?}", interface_desc);
                for endpoint_desc in interface_desc.endpoint_descriptors() {
                    // println!("{:#?}", endpoint_desc);
                    endpoints.push(Endpoint {
                        config: config_desc.number(),
                        iface: interface_desc.interface_number(),
                        setting: interface_desc.setting_number(),
                        address: endpoint_desc.address(),
                    });
                }
            }
        }
    }

    Ok(endpoints)
}

fn configure_endpoint<T: UsbContext>(
    handle: &mut DeviceHandle<T>,
    endpoint: &Endpoint,
) -> Result<()> {
    handle.set_active_configuration(endpoint.config)?;
    handle.claim_interface(endpoint.iface)?;
    handle.set_alternate_setting(endpoint.iface, endpoint.setting)
}

fn set_idle<T: UsbContext>(handle: &mut DeviceHandle<T>) -> Result<usize> {
    let timeout = Duration::from_secs(1);
    const REQEST_TYPE: u8 = 0x21;
    const REQUEST: u8 = 0x0A;
    const VALUE: u16 = 0x0000;
    const INDEX: u16 = 0x0000;
    // // set IDLE request
    handle.write_control(REQEST_TYPE, REQUEST, VALUE, INDEX, &[], timeout)
}

fn set_report<T: UsbContext>(handle: &mut DeviceHandle<T>) -> Result<usize> {
    let timeout = Duration::from_secs(1);

    // values are picked directly from the captured packet
    const REQEST_TYPE: u8 = 0x21;
    const REQUEST: u8 = 0x09;
    const VALUE: u16 = 0x0200;
    const INDEX: u16 = 0x0000;
    const DATA: [u8; 64] = [
        0x3f, 0x10, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x5b,
    ];

    handle.write_control(REQEST_TYPE, REQUEST, VALUE, INDEX, &DATA, timeout)
}

fn read_interrupt<T: UsbContext>(handle: &mut DeviceHandle<T>, address: u8) -> Result<Vec<u8>> {
    let timeout = Duration::from_secs(1);
    let mut buf = [0u8; 64];

    handle
        .read_interrupt(address, &mut buf, timeout)
        .map(|_| buf.to_vec())
}

fn print_data(data: Vec<u8>) {
    let mut rdr = Cursor::new(data);

    // following tweak is suggest by https://www.reddit.com/user/wmanley
    // https://www.reddit.com/r/rust/comments/i0moov/reverse_engineering_a_usb_device_with_rust/fzqflcx?utm_source=share&utm_medium=web2x
    rdr.set_position(7);
    println!(
        "Temp : {:.2}°C",
        rdr.read_u16::<LittleEndian>().unwrap_or_default() as f32 / 256.0
    );

    rdr.set_position(15);
    println!(
        "Fan 1: {:?} rpm",
        rdr.read_u16::<LittleEndian>().unwrap_or_default()
    );

    rdr.set_position(22);
    println!(
        "Fan 2: {:?} rpm",
        rdr.read_u16::<LittleEndian>().unwrap_or_default()
    );

    rdr.set_position(43);
    println!(
        "Fan 3: {:?} rpm",
        rdr.read_u16::<LittleEndian>().unwrap_or_default()
    );

    rdr.set_position(29);
    println!(
        "Pump : {:?} rpm",
        rdr.read_u16::<LittleEndian>().unwrap_or_default()
    );
}
