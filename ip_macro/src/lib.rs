use proc_macro::TokenStream;
use quote::quote;
use std::net::IpAddr;
use syn::{LitStr, parse_macro_input};

#[proc_macro]
pub fn ip(item: TokenStream) -> TokenStream {
    let str = parse_macro_input!(item as LitStr);
    match str.value().parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            let bits = ip.to_bits();
            quote! {
                const { ::core::net::IpAddr::V4(::core::net::Ipv4Addr::from_bits(#bits)) }
            }
        }
        Ok(IpAddr::V6(ip)) => {
            let bits = ip.to_bits();
            quote! {
                const { ::core::net::IpAddr::V6(::core::net::Ipv6Addr::from_bits(#bits)) }
            }
        }
        Err(_) => syn::Error::new_spanned(str, "Invalid ip address").into_compile_error(),
    }
    .into()
}
