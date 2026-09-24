use super::address::DaliAddress;

#[derive(Debug, Clone)]
pub struct Route {
    pub target: DaliAddress,
    pub expects_reply: bool,
}

impl Route {
    pub fn new(target: DaliAddress, is_query: bool) -> Self {
        let expects_reply = is_query && matches!(target, DaliAddress::Short(_));
        Self {
            target,
            expects_reply,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_query_route_expects_reply() {
        let route = Route::new(DaliAddress::short(7).expect("short"), true);
        assert!(route.expects_reply);
    }

    #[test]
    fn group_and_broadcast_routes_never_expect_reply() {
        assert!(!Route::new(DaliAddress::group(3).expect("group"), true).expects_reply);
        assert!(!Route::new(DaliAddress::Broadcast, true).expects_reply);
        assert!(!Route::new(DaliAddress::short(1).expect("short"), false).expects_reply);
    }
}
