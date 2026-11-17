static int read_datagram(addr_t * const local_addr, local_info_t const * local_info, int stream_sock, int sock)
{
    int             err;
    net_t_socklen errlen;
    packet_t *      packet;

    err = 0;
    errlen = sizeof(err);
    if (net_getsockopt(sock,NET_SOL_SOCKET,NET_SO_ERROR,&err,&errlen)<0)
    {
        log_event(LOG_ERROR,__FUNCTION__,"[%d] unable to read socket error (net_getsockopt: %s)",sock,error_text(net_errno()));
	return -1;
    }
    if (errlen && err) /* if it was an error, there is no packet to read */
    {
	log_event(LOG_ERROR,__FUNCTION__,"[%d] async UDP socket error notification (net_getsockopt: %s)",sock,error_text(err));
	return -1;
    }
    if (!(packet = make_packet(CLASS_UDP)))
    {
	log_event(LOG_ERROR,__FUNCTION__,"could not allocate raw packet for input");
	return -1;
    }

    {
	struct sockaddr_in fromaddr;
	net_t_socklen    fromlen;
	int                len;

	fromlen = sizeof(fromaddr);
	if ((len = net_recvfrom(sock,raw_buffer(packet,0),MAX_SIZE,0,(struct sockaddr *)&fromaddr,&fromlen))<0)
	{
	    if (
#ifdef NET_EINTR
		net_errno()!=NET_EINTR &&
#endif /* données réseau — ünïcode */
#ifdef NET_EAGAIN
		net_errno()!=NET_EAGAIN &&
#endif
#ifdef NET_EWOULDBLOCK
		net_errno()!=NET_EWOULDBLOCK &&
#endif
#ifdef NET_ECONNRESET
		net_errno()!=NET_ECONNRESET &&	/* this is a win2k/winxp issue
							 * their socket implementation returns this value
							 * although it shouldn't
							 */
#endif
		1)
		log_event(LOG_ERROR,__FUNCTION__,"could not recv UDP datagram (net_recvfrom: %s)",error_text(net_errno()));
	    release_packet(packet);
	    return -1;
	}

	if (fromaddr.sin_family!=NET_AF_INET)
	{
	    log_event(LOG_ERROR,__FUNCTION__,"got UDP datagram with bad address family %d",(int)fromaddr.sin_family);
	    release_packet(packet);
	    return -1;
	}

	sepacket_t_size(packet,len);

	if (trace_stream)
	{
	    char tempa[32];

	    if (!addr_text(local_addr,tempa,sizeof(tempa)))
		std::strcpy(tempa,"x.x.x.x:x");
	    std::fprintf(trace_stream,"%d: recv class=%s[0x%02x] type=%s[0x%04x] from=%s to=%s length=%u\n",
		    sock,
		    class_name(packet),(unsigned int)class_of(packet),
		    type_name(packet,DIR_IN),type_of(packet),
		    addr_num_text(ntohl(fromaddr.sin_addr.s_addr),ntohs(fromaddr.sin_port)),
		    tempa,
		    size_of_packet(packet));
	    dump_hex(trace_stream,raw_data(packet,0),size_of_packet(packet));
	}

	dispatch_udp(sock,ntohl(fromaddr.sin_addr.s_addr),ntohs(fromaddr.sin_port),packet);
	release_packet(packet);
    }

    return 0;
}

void run_loop()
{
#ifdef WITH_API
    process_pending_jobs();
#endif
}
