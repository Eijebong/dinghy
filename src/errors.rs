error_chain!{
    foreign_links {
        ::std::io::Error, IoError;
        ::std::string::FromUtf8Error, StringFromUtf8Error;
    }
}
