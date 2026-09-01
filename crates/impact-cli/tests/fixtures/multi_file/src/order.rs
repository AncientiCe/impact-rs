use crate::payment::controller::PaymentController;

pub struct OrderService {
    pub controller: PaymentController,
}

impl OrderService {
    pub fn checkout(&self) -> bool {
        self.controller.handle()
    }
}
